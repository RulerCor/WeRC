//! Fully decode bounded GIF assets before they enter the native sender.
use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{io::Cursor, num::NonZeroU64};

pub const MAX_EMOTICON_BYTES: usize = 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmoticonInfo {
    pub width: u16,
    pub height: u16,
    pub frames: u16,
    pub duration_ms: u64,
}
fn invalid() -> AppError {
    AppError::new(
        "UNSUPPORTED_EMOTICON",
        "表情须为完整 GIF，最多1MiB、1024边长、120帧、60秒及3200万解码像素",
    )
}
fn skip(bytes: &[u8], at: &mut usize, count: usize) -> AppResult<()> {
    *at = at
        .checked_add(count)
        .filter(|n| *n <= bytes.len())
        .ok_or_else(invalid)?;
    Ok(())
}
fn subblocks(bytes: &[u8], at: &mut usize) -> AppResult<()> {
    loop {
        let count = usize::from(*bytes.get(*at).ok_or_else(invalid)?);
        skip(bytes, at, 1 + count)?;
        if count == 0 {
            return Ok(());
        }
    }
}
// Validate the complete envelope and cumulative work before LZW decoding.
// A decoder may stop at the first trailer; appended payloads are not accepted.
fn envelope(bytes: &[u8], max_bytes:usize, max_edge:u16) -> AppResult<()> {
    if bytes.len() < 14
        || bytes.len() > max_bytes
        || !matches!(bytes.get(..6), Some(b"GIF87a" | b"GIF89a"))
    {
        return Err(invalid());
    }
    let word = |at| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let (width, height) = (word(6), word(8));
    if width == 0 || height == 0 || width > max_edge || height > max_edge {
        return Err(invalid());
    }
    let mut at = 13;
    if bytes[10] & 0x80 != 0 {
        skip(bytes, &mut at, 3 << (usize::from(bytes[10] & 7) + 1))?;
    }
    let (mut frames, mut pixels) = (0u32, 0u64);
    loop {
        match *bytes.get(at).ok_or_else(invalid)? {
            0x3b if at + 1 == bytes.len() && frames > 0 => return Ok(()),
            0x21 => {
                let label = *bytes.get(at + 1).ok_or_else(invalid)?;
                if !matches!(label, 0xf9 | 0xfe | 0xff) {
                    return Err(invalid());
                }
                skip(bytes, &mut at, 2)?;
                subblocks(bytes, &mut at)?;
            }
            0x2c => {
                let start = at;
                skip(bytes, &mut at, 10)?;
                let (left, top, w, h) = (
                    word(start + 1),
                    word(start + 3),
                    word(start + 5),
                    word(start + 7),
                );
                if w == 0
                    || h == 0
                    || u32::from(left) + u32::from(w) > u32::from(width)
                    || u32::from(top) + u32::from(h) > u32::from(height)
                {
                    return Err(invalid());
                }
                frames += 1;
                pixels += u64::from(w) * u64::from(h);
                if frames > 120 || pixels > 32_000_000 {
                    return Err(invalid());
                }
                let packed = bytes[start + 9];
                if packed & 0x80 != 0 {
                    skip(bytes, &mut at, 3 << (usize::from(packed & 7) + 1))?;
                }
                if !matches!(bytes.get(at), Some(2..=8)) {
                    return Err(invalid());
                }
                skip(bytes, &mut at, 1)?;
                subblocks(bytes, &mut at)?;
            }
            _ => return Err(invalid()),
        }
    }
}
pub fn inspect_gif(name: &str, bytes: &[u8]) -> AppResult<EmoticonInfo> {
    crate::file::validate_file_name(name)?;
    if !name.to_ascii_lowercase().ends_with(".gif") {
        return Err(invalid());
    }
    inspect(bytes, MAX_EMOTICON_BYTES, 1024, None)
}
/// Article images are not limited to the small native sending sticker format.
pub fn inspect_article_gif(bytes:&[u8],cancel:&std::sync::atomic::AtomicBool)->AppResult<EmoticonInfo>{
    inspect(bytes,crate::image::MAX_IMAGE_BYTES,4096,Some(cancel)).map_err(|e|if e.code=="QUERY_CANCELED"{e}else{AppError::new("ARTICLE_GIF_INVALID","文章GIF不完整或超过8MiB、4096边长、120帧、60秒或3200万解码像素")})
}
fn inspect(bytes:&[u8], max_bytes:usize, max_edge:u16, cancel:Option<&std::sync::atomic::AtomicBool>)->AppResult<EmoticonInfo>{
    envelope(bytes,max_bytes,max_edge)?;
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        NonZeroU64::new(32 * 1024 * 1024).unwrap(),
    ));
    options.check_frame_consistency(true);
    options.check_lzw_end_code(true);
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .map_err(|_| invalid())?;
    let mut info = EmoticonInfo {
        width: decoder.width(),
        height: decoder.height(),
        frames: 0,
        duration_ms: 0,
    };
    while let Some(frame) = decoder.read_next_frame().map_err(|_| invalid())? {
        if cancel.is_some_and(|c|c.load(std::sync::atomic::Ordering::SeqCst)){return Err(AppError::new("QUERY_CANCELED","图片读取已取消"));}
        info.frames += 1;
        info.duration_ms += u64::from(frame.delay.max(1)) * 10;
        if info.frames > 120 || info.duration_ms > 60_000 {
            return Err(invalid());
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNED: &[u8] = include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif");
    #[test]
    fn full_frames_and_exact_envelope() {
        let info = inspect_gif("自动化测试.GIF", OWNED).unwrap();
        assert_eq!((info.width, info.height, info.frames), (320, 180, 2));
        assert_eq!(info.duration_ms, 1600);
        assert!(inspect_gif("wrong.mp4", OWNED).is_err());
        let mut tail = OWNED.to_vec();
        tail.extend_from_slice(b"hidden\x3b");
        assert!(inspect_gif("x.gif", &tail).is_err());
        assert!(inspect_gif("x.gif", &OWNED[..OWNED.len() - 1]).is_err());
        let mut huge = OWNED.to_vec();
        huge[6..8].copy_from_slice(&1025u16.to_le_bytes());
        assert!(inspect_gif("x.gif", &huge).is_err());
    }
    #[test]
    fn every_truncated_prefix_is_rejected_without_panicking() {
        for n in 0..OWNED.len() {
            assert!(inspect_gif("x.gif", &OWNED[..n]).is_err(), "prefix {n}");
        }
    }
}
