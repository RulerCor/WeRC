//! Version-0 cached animation: HEVC pictures, JBIG transparency and millisecond timing.
//! Only the verified profile is accepted; checksums identify this cache representation,
//! not the original GIF named by the message. Sending keeps its separate GIF limits.
use crate::{AppError, AppResult, emoticon::EmoticonInfo};
use md5::{Digest, Md5};
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_PIXELS: u64 = 48_000_000;
pub const MAX_PREVIEW_BYTES: usize = 32 * 1024 * 1024;
pub const FOOTER: &[u8; 8] = b"\x92\xe5\xbd\xbd\0\0\0\0";
pub struct Frame<'a> {
    pub duration_ms: u16,
    pub alpha: &'a [u8],
    pub picture: &'a [u8],
}
pub struct Animation<'a> {
    pub info: EmoticonInfo,
    pub frames: Vec<Frame<'a>>,
}
fn invalid() -> AppError {
    AppError::new(
        "WXGF_UNSUPPORTED",
        "此缓存动画格式不受支持、校验失败或超过接收限额",
    )
}
struct Input<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Input<'a> {
    fn take(&mut self, n: usize) -> AppResult<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or_else(invalid)?;
        let v = self.bytes.get(self.at..end).ok_or_else(invalid)?;
        self.at = end;
        Ok(v)
    }
    fn word(&mut self) -> AppResult<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn block(&mut self) -> AppResult<&'a [u8]> {
        let n = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
        self.take(n)
    }
}
pub fn parse(bytes: &[u8]) -> AppResult<Animation<'_>> {
    if bytes.len() < 42 || bytes.len() > MAX_BYTES || !bytes.starts_with(b"wxgf") {
        return Err(invalid());
    }
    let end = bytes.len() - 24;
    if &bytes[end..end + 8] != FOOTER || Md5::digest(&bytes[..end]).as_slice() != &bytes[end + 8..]
    {
        return Err(invalid());
    }
    let header = usize::from(bytes[4]);
    if !(16..=32).contains(&header) || header > end {
        return Err(invalid());
    }
    let mut input = Input {
        bytes: &bytes[..end],
        at: 5,
    };
    if input.word()? != 0 {
        return Err(invalid());
    }
    let (width, height, count, loops) = (
        input.word()?,
        input.word()?,
        input.word()? as usize + 1,
        input.word()?,
    );
    if width == 0
        || height == 0
        || width > 1024
        || height > 1024
        || width % 2 != 0
        || height % 2 != 0
        || count > 120
        || u64::from(width) * u64::from(height) * count as u64 > MAX_PIXELS
        || loops != 0
    {
        return Err(invalid());
    }
    let mut bit = 120usize;
    let mut bits = |n: usize| -> AppResult<u32> {
        if bit + n > header * 8 {
            return Err(invalid());
        }
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | u32::from((bytes[bit / 8] >> (7 - bit % 8)) & 1);
            bit += 1;
        }
        Ok(v)
    };
    if bits(1)? != 1 || bits(7)? != 1 {
        return Err(invalid());
    }
    let duration = if bits(1)? != 0 {
        Some(bits(16)? as u16)
    } else {
        None
    };
    let codec = if bits(1)? != 0 {
        Some(bits(2)? as u8)
    } else {
        None
    };
    let palette = if bits(1)? != 0 {
        let n = bits(4)?;
        if n > 8 {
            return Err(invalid());
        }
        3usize << n
    } else {
        0
    };
    if bit.div_ceil(8) != header {
        return Err(invalid());
    }
    input.at = header;
    input.take(palette)?;
    let mut frames = Vec::with_capacity(count);
    let mut duration_ms = 0u64;
    let mut picture_bytes = 0usize;
    for _ in 0..count {
        let duration_ms_frame = match duration {
            Some(v) => v,
            None => input.word()?,
        };
        duration_ms += u64::from(duration_ms_frame);
        if duration_ms_frame == 0 || duration_ms > 60_000 {
            return Err(invalid());
        }
        let codec = match codec {
            Some(v) => v,
            None => input.take(1)?[0],
        };
        if codec != 1 {
            return Err(invalid());
        }
        let alpha = input.block()?;
        if alpha.len() < 20
            || alpha[..4] != [0, 0, 1, 0]
            || u32::from_be_bytes(alpha[4..8].try_into().unwrap()) != u32::from(width)
            || u32::from_be_bytes(alpha[8..12].try_into().unwrap()) != u32::from(height)
        {
            return Err(invalid());
        }
        let picture = input.block()?;
        picture_bytes += picture.len();
        frames.push(Frame {
            duration_ms: duration_ms_frame,
            alpha,
            picture,
        });
    }
    if input.at != end || picture_bytes == 0 {
        return Err(invalid());
    }
    Ok(Animation {
        info: EmoticonInfo {
            width,
            height,
            frames: count as u16,
            duration_ms,
        },
        frames,
    })
}

/// Encode the isolated decoder's complete RGBA stream, retaining per-frame timing.
/// Output is a derived APNG preview; the original cached bytes remain separately stored.
pub fn encode_preview(
    mut input: impl std::io::Read,
    info: &EmoticonInfo,
    delays: &[u16],
) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
    use std::io::{self, Write};
    const MAGIC: u32 = 0x45525657;
    if delays.len() != usize::from(info.frames)
        || delays.is_empty()
        || delays.len() > 120
        || info.width == 0
        || info.height == 0
        || info.width > 1024
        || info.height > 1024
        || u64::from(info.width) * u64::from(info.height) * u64::from(info.frames) > MAX_PIXELS
        || delays.contains(&0)
        || delays.iter().map(|d| u64::from(*d)).sum::<u64>() != info.duration_ms
        || info.duration_ms > 60_000
    {
        return Err(invalid());
    }
    fn word(r: &mut impl io::Read) -> AppResult<u32> {
        let mut b = [0; 4];
        r.read_exact(&mut b).map_err(|_| invalid())?;
        Ok(u32::from_le_bytes(b))
    }
    for expected in [
        MAGIC,
        1,
        u32::from(info.width),
        u32::from(info.height),
        u32::from(info.frames),
    ] {
        if word(&mut input)? != expected {
            return Err(invalid());
        }
    }
    struct Output(zeroize::Zeroizing<Vec<u8>>);
    impl Write for Output {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            if b.len() > MAX_PREVIEW_BYTES - self.0.len() {
                return Err(io::Error::other("preview limit"));
            }
            self.0.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut out = Output(zeroize::Zeroizing::new(Vec::new()));
    let mut encoder = png::Encoder::new(&mut out, u32::from(info.width), u32::from(info.height));
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .set_animated(u32::from(info.frames), 0)
        .map_err(|_| invalid())?;
    let mut writer = encoder.write_header().map_err(|_| invalid())?;
    let mut rgba =
        zeroize::Zeroizing::new(vec![
            0;
            usize::from(info.width) * usize::from(info.height) * 4
        ]);
    for delay in delays {
        input.read_exact(&mut rgba).map_err(|_| invalid())?;
        writer
            .set_frame_delay(*delay, 1000)
            .map_err(|_| invalid())?;
        writer
            .set_blend_op(png::BlendOp::Source)
            .map_err(|_| invalid())?;
        writer
            .set_dispose_op(png::DisposeOp::None)
            .map_err(|_| invalid())?;
        writer.write_image_data(&rgba).map_err(|_| invalid())?;
    }
    if word(&mut input)? != MAGIC || word(&mut input)? != u32::from(info.frames) {
        return Err(invalid());
    }
    let mut extra = [0];
    if input.read(&mut extra).map_err(|_| invalid())? != 0 {
        return Err(invalid());
    }
    writer.finish().map_err(|_| invalid())?;
    Ok(out.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn checksum(v: &mut Vec<u8>) {
        v.extend_from_slice(FOOTER);
        let h = Md5::digest(&v[..v.len() - 8]);
        v.extend_from_slice(&h);
    }
    fn sample() -> Vec<u8> {
        let mut v = b"wxgf\x11\0\0\0\x10\0\x10\0\0\0\0\x81\x50".to_vec();
        v.extend_from_slice(&40u16.to_be_bytes());
        v.extend_from_slice(&20u32.to_be_bytes());
        v.extend_from_slice(&[0, 0, 1, 0]);
        v.extend_from_slice(&16u32.to_be_bytes());
        v.extend_from_slice(&16u32.to_be_bytes());
        v.extend_from_slice(&[0, 0, 0, 8, 0, 0, 0, 0]);
        v.extend_from_slice(&7u32.to_be_bytes());
        v.extend_from_slice(&[0, 0, 0, 1, 0x40, 1, 0]);
        checksum(&mut v);
        v
    }
    #[test]
    fn preview_preserves_full_alpha_timing_and_rejects_partial_stream() {
        let info = EmoticonInfo {
            width: 2,
            height: 2,
            frames: 2,
            duration_ms: 70,
        };
        let mut raw = Vec::new();
        for word in [0x45525657u32, 1, 2, 2, 2] {
            raw.extend_from_slice(&word.to_le_bytes());
        }
        raw.extend_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 255]);
        raw.extend_from_slice(&[0, 0, 0, 0, 10, 20, 30, 255, 30, 20, 10, 255, 0, 0, 0, 0]);
        for word in [0x45525657u32, 2] {
            raw.extend_from_slice(&word.to_le_bytes());
        }
        let out = encode_preview(std::io::Cursor::new(&raw), &info, &[30, 40]).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(out.as_slice()))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().animation_control.unwrap().num_frames, 2);
        let mut pixels = [0; 16];
        for (n, delay) in [30, 40].into_iter().enumerate() {
            reader.next_frame(&mut pixels).unwrap();
            assert_eq!(&pixels[..], &raw[20 + n * 16..36 + n * 16]);
            let fc = reader.info().frame_control.unwrap();
            assert_eq!((fc.delay_num, fc.delay_den), (delay, 1000));
        }
        for n in 0..raw.len() {
            assert!(encode_preview(std::io::Cursor::new(&raw[..n]), &info, &[30, 40]).is_err());
        }
        raw.push(1);
        assert!(encode_preview(std::io::Cursor::new(&raw), &info, &[30, 40]).is_err());
    }
    #[test]
    fn exact_layout_and_footer() {
        let v = sample();
        let a = parse(&v).unwrap();
        assert_eq!(
            (
                a.info.width,
                a.info.height,
                a.info.frames,
                a.info.duration_ms
            ),
            (16, 16, 1, 40)
        );
        assert_eq!(a.frames[0].picture.len(), 7);
    }
    #[test]
    fn all_truncated_and_corrupted_bytes_rejected() {
        let v = sample();
        for n in 0..v.len() {
            assert!(parse(&v[..n]).is_err());
        }
        for i in 0..v.len() {
            let mut b = v.clone();
            b[i] ^= 1;
            assert!(parse(&b).is_err());
        }
    }
    #[test]
    fn valid_checksum_does_not_bypass_profile_and_budgets() {
        let original = sample();
        for (at, value) in [
            (5, 1),
            (8, 17),
            (11, 1),
            (14, 1),
            (15, 0),
            (16, 0),
            (17, 0xff),
            (19, 0xff),
            (24, 2),
        ] {
            let mut v = original[..original.len() - 24].to_vec();
            v[at] = value;
            checksum(&mut v);
            assert!(parse(&v).is_err(), "{at}");
        }
    }
    #[test]
    fn extra_frames_and_data_rejected() {
        let mut v = sample();
        v.truncate(v.len() - 24);
        v.push(0);
        checksum(&mut v);
        assert!(parse(&v).is_err());
    }
}
