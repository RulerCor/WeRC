//! A one-request codec process. No audio is written to disk or a command line.
use sha2::{Digest, Sha256};
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    process::CommandExt,
};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use werc_domain::{
    AppError, AppResult,
    voice::{EncodedVoice, MAX_SILK_BYTES, silk_duration, voice_pcm},
};
use zeroize::Zeroizing;
const MAGIC: u32 = 0x43525657;
fn error() -> AppError {
    AppError::new(
        "VOICE_CODEC_FAILED",
        "语音编码组件未通过完整性、协议或时限检查；没有发送",
    )
}

pub fn encode(wav: &[u8]) -> AppResult<EncodedVoice> {
    let exe = std::env::current_exe().map_err(|_| error())?;
    encode_at(
        &exe.parent().ok_or_else(error)?.join("werc-voice-codec.exe"),
        option_env!("WERC_VOICE_CODEC_SHA256").unwrap_or(""),
        wav,
    )
}
pub fn encode_at(exe: &Path, expected: &str, wav: &[u8]) -> AppResult<EncodedVoice> {
    let pcm = voice_pcm(wav)?;
    let samples = pcm.len() / 2;
    let bytes = run(exe, expected, 1, pcm, samples)?;
    let duration = silk_duration(&bytes)?;
    if duration as usize != samples / 16 {
        return Err(error());
    }
    Ok(EncodedVoice {
        bytes,
        duration_ms: duration,
        codec_sha256: expected.into(),
    })
}
pub fn decode(silk: &[u8]) -> AppResult<Zeroizing<Vec<u8>>> {
    let exe = std::env::current_exe().map_err(|_| error())?;
    decode_at(
        &exe.parent().ok_or_else(error)?.join("werc-voice-codec.exe"),
        option_env!("WERC_VOICE_CODEC_SHA256").unwrap_or(""),
        silk,
    )
}
pub fn decode_at(exe: &Path, expected: &str, silk: &[u8]) -> AppResult<Zeroizing<Vec<u8>>> {
    let duration = silk_duration(silk)?;
    let pcm = run(
        exe,
        expected,
        2,
        Zeroizing::new(silk.to_vec()),
        duration as usize * 16,
    )?;
    if pcm.len() != duration as usize * 32 {
        return Err(error());
    }
    let n = pcm.len() as u32;
    let mut wav = Zeroizing::new(Vec::with_capacity(pcm.len() + 44));
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(n + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&32000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&n.to_le_bytes());
    wav.extend_from_slice(&pcm);
    werc_domain::media::inspect_wav(&wav)?;
    Ok(wav)
}
pub(crate) fn available() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(directory) = exe.parent() else {
        return false;
    };
    pin_codec(
        &directory.join("werc-voice-codec.exe"),
        option_env!("WERC_VOICE_CODEC_SHA256").unwrap_or(""),
    )
    .is_ok()
}
fn pin_codec(exe: &Path, expected: &str) -> AppResult<std::fs::File> {
    if !exe.is_absolute()
        || exe.file_name().and_then(|s| s.to_str()) != Some("werc-voice-codec.exe")
        || expected.len() != 64
    {
        return Err(error());
    }
    for p in exe.ancestors() {
        if std::fs::symlink_metadata(p)
            .map_err(|_| error())?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err(error());
        }
    }
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(exe)
        .map_err(|_| error())?;
    if file.metadata().map_err(|_| error())?.len() > 16 * 1024 * 1024 {
        return Err(error());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| error())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if hex::encode(hash.finalize()) != expected {
        return Err(error());
    }
    Ok(file)
}
fn run(
    exe: &Path,
    expected: &str,
    op: u32,
    input: Zeroizing<Vec<u8>>,
    samples: usize,
) -> AppResult<Zeroizing<Vec<u8>>> {
    // Retain the deny-write/delete handle for the entire child invocation.
    let _image = pin_codec(exe, expected)?;
    let mut child = Command::new(exe)
        .current_dir(exe.parent().ok_or_else(error)?)
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error())?;
    let job = match crate::native_messages::Job::attach(&child) {
        Ok(j) => j,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error());
        }
    };
    let started = Instant::now();
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::sync_channel(1);
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for v in [MAGIC, 1, op, 16000, input.len() as u32] {
            stdin.write_all(&v.to_le_bytes())?;
        }
        stdin.write_all(&input)?;
        drop(stdin);
        Ok(())
    });
    let reader = std::thread::spawn(move || {
        let result = (|| -> AppResult<Zeroizing<Vec<u8>>> {
            let mut h = [0u8; 24];
            stdout.read_exact(&mut h).map_err(|_| error())?;
            let word = |i| u32::from_le_bytes(h[i..i + 4].try_into().expect("fixed header"));
            let n = word(20) as usize;
            let max = if op == 1 { MAX_SILK_BYTES } else { 960000 * 2 };
            if word(0) != MAGIC
                || word(4) != 1
                || word(8) != 0
                || word(12) != 16000
                || word(16) as usize != samples
                || n == 0
                || n > max
            {
                return Err(error());
            }
            let mut out = Zeroizing::new(vec![0; n]);
            stdout.read_exact(&mut out).map_err(|_| error())?;
            let mut extra = [0];
            if stdout.read(&mut extra).map_err(|_| error())? != 0 {
                return Err(error());
            }
            Ok(out)
        })();
        let _ = tx.send(result);
    });
    let mut result = rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| error())
        .and_then(|r| r);
    let exited = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.success(),
            Err(_) => break false,
            _ => {}
        }
        if result.is_err() || started.elapsed() >= Duration::from_secs(30) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !exited {
        let _ = child.kill();
        result = Err(error());
    }
    let _ = child.wait();
    drop(job);
    if !matches!(writer.join(), Ok(Ok(()))) || reader.join().is_err() {
        return Err(error());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a source-built codec path and hash"]
    fn source_built_worker_roundtrip_and_hash_rejection() {
        let path = std::env::var("WERC_TEST_VOICE_CODEC").expect("source-built codec");
        let hash = hex::encode(Sha256::digest(std::fs::read(&path).unwrap()));
        let mut pcm = Zeroizing::new(Vec::new());
        for i in 0..16000 {
            let s = ((i as f64 * 440.0 * std::f64::consts::TAU / 16000.0).sin() * 12000.0) as i16;
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        assert!(run(Path::new(&path), &"0".repeat(64), 1, pcm.clone(), 16000).is_err());
        let silk = run(Path::new(&path), &hash, 1, pcm, 16000).unwrap();
        assert_eq!(silk_duration(&silk).unwrap(), 1000);
        let wav = decode_at(Path::new(&path), &hash, &silk).unwrap();
        assert_eq!(
            werc_domain::media::inspect_wav(&wav).unwrap().duration_ms,
            1000
        );
        let restored = run(Path::new(&path), &hash, 2, silk, 16000).unwrap();
        assert_eq!(restored.len(), 32000);
        assert!(restored.iter().any(|b| *b != 0));
        assert!(
            run(
                Path::new(&path),
                &hash,
                2,
                Zeroizing::new(vec![0; 16]),
                16000
            )
            .is_err()
        );
    }
}
