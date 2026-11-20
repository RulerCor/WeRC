//! Narrow native voice format: 16 kHz mono PCM16, complete 20 ms packets.
use crate::{AppError, AppResult, media::inspect_wav};
use zeroize::Zeroizing;

pub const MAX_SILK_BYTES: usize = 512 * 1024;
pub fn invalid() -> AppError {
    AppError::new(
        "VOICE_FORMAT_INVALID",
        "原生语音需16kHz单声道PCM16、20毫秒整帧，最长60秒",
    )
}
pub fn voice_pcm(wav: &[u8]) -> AppResult<Zeroizing<Vec<u8>>> {
    let info = inspect_wav(wav)?;
    if info.sample_rate != 16000
        || info.channels != 1
        || info.frames > 960000
        || info.frames % 320 != 0
    {
        return Err(invalid());
    }
    let mut at = 12;
    while at < wav.len() {
        // inspect_wav has authenticated the complete RIFF chunk structure.
        let n = u32::from_le_bytes(wav[at + 4..at + 8].try_into().map_err(|_| invalid())?) as usize;
        if &wav[at..at + 4] == b"data" {
            return Ok(Zeroizing::new(wav[at + 8..at + 8 + n].to_vec()));
        }
        at += 8 + n + n % 2;
    }
    Err(invalid())
}
pub fn silk_duration(bytes: &[u8]) -> AppResult<u32> {
    if bytes.len() <= 12 || bytes.len() > MAX_SILK_BYTES || !bytes.starts_with(b"\x02#!SILK_V3") {
        return Err(invalid());
    }
    let mut at = 10;
    let mut packets = 0;
    while at < bytes.len() {
        let pair = bytes.get(at..at + 2).ok_or_else(invalid)?;
        let n = u16::from_le_bytes([pair[0], pair[1]]) as usize;
        at += 2;
        if n == 0 || n > 1024 || n > bytes.len() - at || packets == 3000 {
            return Err(invalid());
        }
        at += n;
        packets += 1;
    }
    Ok(packets * 20)
}
pub struct EncodedVoice {
    pub bytes: Zeroizing<Vec<u8>>,
    pub duration_ms: u32,
    pub codec_sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silk_rejects_truncation_and_packet_overflow() {
        let good = b"\x02#!SILK_V3\x02\x00\x01\x01";
        assert_eq!(silk_duration(good).unwrap(), 20);
        for n in 0..good.len() {
            assert!(silk_duration(&good[..n]).is_err());
        }
        let mut b = good.to_vec();
        b[10] = 0;
        b[11] = 0;
        assert!(silk_duration(&b).is_err());
        let mut b = b"\x02#!SILK_V3".to_vec();
        for _ in 0..3001 {
            b.extend_from_slice(&[1, 0, 1]);
        }
        assert!(silk_duration(&b).is_err());
    }
}
