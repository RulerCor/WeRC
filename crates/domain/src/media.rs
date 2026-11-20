use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};

pub const MAX_AUDIO_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ACCOUNT_ASSET_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioInfo {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub frames: u64,
    pub duration_ms: u64,
    pub peak: f64,
    pub rms: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioAsset {
    pub id: String,
    pub account_id: String,
    pub sha256: String,
    pub size: usize,
    pub audio: AudioInfo,
    pub created_at: String,
    pub expires_at: String,
}

// 仅保存密文和受保护的随机数据密钥，不实现 Debug/Serialize。
pub struct ProtectedAsset {
    pub asset: AudioAsset,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub fn inspect_wav(bytes: &[u8]) -> AppResult<AudioInfo> {
    let invalid = || {
        AppError::new(
            "UNSUPPORTED_MEDIA",
            "仅支持完整的 RIFF/WAVE PCM16 音频；没有发送任何消息",
        )
    };
    if bytes.len() > MAX_AUDIO_BYTES {
        return Err(AppError::new("QUOTA_EXCEEDED", "单个音频最多 8 MiB"));
    }
    if bytes.len() < 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid());
    }
    let u32le = |offset| {
        u32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("已检查分块边界"),
        )
    };
    if u32le(4) as u64 + 8 != bytes.len() as u64 {
        return Err(invalid());
    }
    let mut offset = 12usize;
    let mut format = None;
    let mut samples = None;
    let mut chunks = 0;
    while offset < bytes.len() {
        if bytes.len() - offset < 8 || chunks >= 256 {
            return Err(invalid());
        }
        chunks += 1;
        let length = u32le(offset + 4) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(length)
            .filter(|n| *n <= bytes.len())
            .ok_or_else(invalid)?;
        match &bytes[offset..offset + 4] {
            b"fmt " => {
                if format.is_some() || !(16..=40).contains(&length) {
                    return Err(invalid());
                }
                let short = |n| {
                    u16::from_le_bytes(
                        bytes[start + n..start + n + 2]
                            .try_into()
                            .expect("fmt长度有效"),
                    )
                };
                let encoding = short(0);
                let channels = short(2);
                let rate = u32le(start + 4);
                let byte_rate = u32le(start + 8);
                let align = short(12);
                let bits = short(14);
                if encoding != 1
                    || !(1..=2).contains(&channels)
                    || !(8000..=96000).contains(&rate)
                    || bits != 16
                    || align != channels * 2
                    || byte_rate != rate * u32::from(align)
                {
                    return Err(invalid());
                }
                format = Some((channels, rate, align));
            }
            b"data" => {
                if samples.is_some() || length == 0 {
                    return Err(invalid());
                }
                samples = Some(&bytes[start..end]);
            }
            _ => {}
        }
        offset = end
            .checked_add(length % 2)
            .filter(|n| *n <= bytes.len())
            .ok_or_else(invalid)?;
    }
    let (channels, rate, align) = format.ok_or_else(invalid)?;
    let samples = samples.ok_or_else(invalid)?;
    if samples.len() % usize::from(align) != 0 {
        return Err(invalid());
    }
    let frames = samples.len() as u64 / u64::from(align);
    if frames == 0 || frames > u64::from(rate) * 600 {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "本地音频处理上限为 10 分钟；不是微信语音时长承诺",
        ));
    }
    let mut peak = 0f64;
    let mut squares = 0f64;
    for pair in samples.as_chunks::<2>().0 {
        let value = f64::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0;
        peak = peak.max(value.abs());
        squares += value * value;
    }
    Ok(AudioInfo {
        format: "wav_pcm16".into(),
        sample_rate: rate,
        channels,
        bits_per_sample: 16,
        frames,
        duration_ms: frames * 1000 / u64::from(rate),
        peak,
        rms: (squares / (samples.len() / 2) as f64).sqrt(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wave() -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend(40u32.to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(16000u32.to_le_bytes());
        b.extend(32000u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(4u32.to_le_bytes());
        b.extend(i16::MIN.to_le_bytes());
        b.extend(0i16.to_le_bytes());
        b
    }
    #[test]
    fn measures_pcm_not_extension() {
        let info = inspect_wav(&wave()).unwrap();
        assert_eq!(info.frames, 2);
        assert_eq!(info.peak, 1.0);
        assert!((info.rms - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
    }
    #[test]
    fn rejects_truncation_and_bad_format() {
        let good = wave();
        for length in 0..good.len() {
            assert!(inspect_wav(&good[..length]).is_err());
        }
        let mut b = good.clone();
        b[20] = 3;
        assert!(inspect_wav(&b).is_err());
        let mut b = good.clone();
        b[32] = 3;
        assert!(inspect_wav(&b).is_err());
        let mut b = good;
        b[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(inspect_wav(&b).is_err());
    }
    #[test]
    fn arbitrary_small_inputs_do_not_panic() {
        for seed in 0..256u16 {
            let b: Vec<u8> = (0..512)
                .map(|n| (n as u16).wrapping_mul(seed) as u8)
                .collect();
            assert!(inspect_wav(&b).is_err());
        }
    }
}
