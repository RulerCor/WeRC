//! Isolated source-built animation decoding. Paths derive from the running component.
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
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use werc_domain::{AppError, AppResult};
use zeroize::Zeroizing;
fn error() -> AppError {
    AppError::new(
        "EMOTICON_CODEC_FAILED",
        "动画解码组件不可用、格式异常或超过处理时限",
    )
}
pub fn render(bytes: &[u8], cancel: &AtomicBool) -> AppResult<Zeroizing<Vec<u8>>> {
    let exe = std::env::current_exe().map_err(|_| error())?;
    render_at(
        &exe.parent()
            .ok_or_else(error)?
            .join("werc-emoticon-codec.exe"),
        option_env!("WERC_EMOTICON_CODEC_SHA256").unwrap_or(""),
        bytes,
        cancel,
    )
}
fn pin_codec(exe: &Path, expected: &str) -> AppResult<std::fs::File> {
    if !exe.is_absolute()
        || exe.file_name().and_then(|s| s.to_str()) != Some("werc-emoticon-codec.exe")
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
pub fn render_at(
    exe: &Path,
    expected: &str,
    bytes: &[u8],
    cancel: &AtomicBool,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let animation = werc_domain::wxgf::parse(bytes)?;
    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::new("QUERY_CANCELED", "动画解码已取消"));
    }
    let _pin = pin_codec(exe, expected)?;
    let mut input = Zeroizing::new(Vec::new());
    for word in [
        0x45525657,
        1,
        u32::from(animation.info.width),
        u32::from(animation.info.height),
        u32::from(animation.info.frames),
    ] {
        input.extend_from_slice(&word.to_le_bytes());
    }
    let mut delays = Vec::new();
    for f in &animation.frames {
        delays.push(f.duration_ms);
        for b in [f.alpha, f.picture] {
            input.extend_from_slice(&(b.len() as u32).to_le_bytes());
            input.extend_from_slice(b);
        }
    }
    let info = animation.info;
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
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let reader = std::thread::spawn(move || {
        let _ = tx.send(werc_domain::wxgf::encode_preview(stdout, &info, &delays));
    });
    let start = Instant::now();
    let mut result = loop {
        if cancel.load(Ordering::SeqCst) {
            break Err(AppError::new("QUERY_CANCELED", "动画解码已取消"));
        }
        if start.elapsed() >= Duration::from_secs(30) {
            break Err(error());
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(r) => break r,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break Err(error()),
        }
    };
    let exited = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.success(),
            Err(_) => break false,
            _ => {}
        }
        if result.is_err()
            || cancel.load(Ordering::SeqCst)
            || start.elapsed() >= Duration::from_secs(30)
        {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !exited {
        let _ = child.kill();
        if result.is_ok() {
            result = Err(error());
        }
    }
    let _ = child.wait();
    drop(job);
    let write_ok = matches!(writer.join(), Ok(Ok(())));
    let read_ok = reader.join().is_ok();
    if result.is_ok() && (!write_ok || !read_ok) {
        return Err(error());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires source-built animation codec"]
    fn source_codec_preview_hash_and_cancel() {
        let path = std::env::var("WERC_TEST_EMOTICON_CODEC").unwrap();
        let path = Path::new(&path);
        let hash = hex::encode(Sha256::digest(std::fs::read(path).unwrap()));
        let input = include_bytes!("../../../tests/fixtures/owned-received-emoticon.wxgf");
        assert!(render_at(path, &"0".repeat(64), input, &AtomicBool::new(false)).is_err());
        assert_eq!(
            render_at(path, &hash, input, &AtomicBool::new(true))
                .unwrap_err()
                .code,
            "QUERY_CANCELED"
        );
        let png = render_at(path, &hash, input, &AtomicBool::new(false)).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() < 8192);
    }
}
