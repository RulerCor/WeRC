//! Bounded static image input, decoded and rewritten without metadata before send.
mod orientation;
use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Write};
use zeroize::Zeroizing;

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 4 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageAsset {
    pub id: String,
    pub account_id: String,
    pub sha256: String,
    pub source_sha256: String,
    pub size: usize,
    pub image: ImageInfo,
    pub created_at: String,
    pub expires_at: String,
}
pub struct ProtectedImage {
    pub asset: ImageAsset,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
fn invalid() -> AppError {
    AppError::new(
        "UNSUPPORTED_IMAGE",
        "仅支持完整静态PNG，最多8MiB、4096像素边长与4194304像素总面积",
    )
}
struct LimitedOutput(Zeroizing<Vec<u8>>);
impl Write for LimitedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_IMAGE_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("image size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// Received images may be transformed by the native CDN client. Validate the
/// completed container and pixel budget, then discard all original metadata.
pub fn canonical_received_png(bytes: &[u8]) -> AppResult<(ImageInfo, Zeroizing<Vec<u8>>)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return canonical_png(bytes);
    }
    canonical_jpeg(bytes, true)
}
/// Local uploads use the same portable decoder, but do not allow CDN footers.
/// The protected asset and native send protocol remain canonical static PNG.
pub fn canonical_upload_png(bytes: &[u8]) -> AppResult<(ImageInfo, Zeroizing<Vec<u8>>)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return canonical_png(bytes);
    }
    canonical_jpeg(bytes, false).map_err(|_| AppError::new(
        "UNSUPPORTED_IMAGE",
        "请选择完整静态PNG或JPEG；最多8MiB、4096像素边长与4194304像素总面积，照片方向信息须有效",
    ))
}
fn canonical_jpeg(bytes: &[u8], allow_footer: bool) -> AppResult<(ImageInfo, Zeroizing<Vec<u8>>)> {
    let error = || {
        AppError::new(
            "UNSUPPORTED_RECEIVED_IMAGE",
            "接收图片仅支持完整JPEG/静态PNG，最多8MiB、4096边长和4194304像素",
        )
    };
    if bytes.len() < 4 || bytes.len() > MAX_IMAGE_BYTES || !bytes.starts_with(&[255, 216]) {
        return Err(error());
    }
    let mut input = Cursor::new(bytes);
    let (info, raw, format, orientation) = {
        let mut decoder = jpeg_decoder::Decoder::new(&mut input);
        decoder.set_max_decoding_buffer_size(16 * 1024 * 1024);
        decoder.read_info().map_err(|_| error())?;
        let meta = decoder.info().ok_or_else(error)?;
        let info = ImageInfo {
            width: u32::from(meta.width),
            height: u32::from(meta.height),
        };
        if info.width == 0
            || info.height == 0
            || info.width > 4096
            || info.height > 4096
            || u64::from(info.width) * u64::from(info.height) > MAX_IMAGE_PIXELS
            || !matches!(
                meta.pixel_format,
                jpeg_decoder::PixelFormat::L8 | jpeg_decoder::PixelFormat::RGB24
            )
        {
            return Err(error());
        }
        let raw = Zeroizing::new(decoder.decode().map_err(|_| error())?);
        let orientation = orientation::read(decoder.exif_data()).map_err(|_| error())?;
        (info, raw, meta.pixel_format, orientation)
    };
    let consumed = input.position() as usize;
    // Native CDN JPEG output can include a short footer (24 bytes in the
    // verified capture). Its meaning is not assumed. Discard at most 64 bytes
    // after a fully decoded EOI; never preserve or expose that opaque footer.
    if consumed < 2
        || consumed > bytes.len()
        || bytes[consumed - 2..consumed] != [255, 217]
        || bytes.len() - consumed > if allow_footer { 64 } else { 0 }
        || raw.len() != info.width as usize * info.height as usize * format.pixel_bytes()
    {
        return Err(error());
    }
    let mut rgba = Zeroizing::new(Vec::with_capacity(
        info.width as usize * info.height as usize * 4,
    ));
    match format {
        jpeg_decoder::PixelFormat::L8 => {
            for &v in raw.iter() {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        jpeg_decoder::PixelFormat::RGB24 => {
            for p in raw.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        _ => return Err(error()),
    }
    let (info, rgba) = orientation::apply(info, rgba, orientation);
    let mut output = LimitedOutput(Zeroizing::new(Vec::new()));
    {
        let mut encoder = png::Encoder::new(&mut output, info.width, info.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|_| error())?;
        writer.write_image_data(&rgba).map_err(|_| error())?;
        writer.finish().map_err(|_| error())?;
    }
    Ok((info, output.0))
}
pub fn canonical_png(bytes: &[u8]) -> AppResult<(ImageInfo, Zeroizing<Vec<u8>>)> {
    if bytes.len() < 45 || bytes.len() > MAX_IMAGE_BYTES || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(invalid());
    }
    // Require a single complete container. Do not silently ignore appended
    // payloads, APNG animation, or an unterminated chunk sequence.
    let mut at = 8usize;
    let mut ended = false;
    while at < bytes.len() {
        if bytes.len() - at < 12 {
            return Err(invalid());
        }
        let size =
            u32::from_be_bytes(bytes[at..at + 4].try_into().map_err(|_| invalid())?) as usize;
        if size > bytes.len() - at - 12 {
            return Err(invalid());
        }
        let kind = &bytes[at + 4..at + 8];
        if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
            return Err(invalid());
        }
        at += size + 12;
        if kind == b"IEND" {
            if size != 0 || at != bytes.len() {
                return Err(invalid());
            }
            ended = true;
            break;
        }
    }
    if !ended {
        return Err(invalid());
    }
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: 32 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    let mut reader = decoder.read_info().map_err(|_| invalid())?;
    let info = ImageInfo {
        width: reader.info().width,
        height: reader.info().height,
    };
    if info.width == 0
        || info.height == 0
        || info.width > 4096
        || info.height > 4096
        || u64::from(info.width) * u64::from(info.height) > MAX_IMAGE_PIXELS
        || reader.info().animation_control.is_some()
    {
        return Err(invalid());
    }
    let size = reader
        .output_buffer_size()
        .filter(|s| *s <= 16 * 1024 * 1024)
        .ok_or_else(invalid)?;
    let mut raw = Zeroizing::new(vec![0; size]);
    let decoded = reader.next_frame(&mut raw).map_err(|_| invalid())?;
    reader.finish().map_err(|_| invalid())?;
    if decoded.bit_depth != png::BitDepth::Eight {
        return Err(invalid());
    }
    raw.truncate(decoded.buffer_size());
    let mut rgba = Zeroizing::new(Vec::with_capacity(
        info.width as usize * info.height as usize * 4,
    ));
    match decoded.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(&raw),
        png::ColorType::Rgb => {
            for pixel in raw.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &v in raw.iter() {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in raw.as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        _ => return Err(invalid()),
    }
    if rgba.len() != info.width as usize * info.height as usize * 4 {
        return Err(invalid());
    }
    let mut output = LimitedOutput(Zeroizing::new(Vec::new()));
    {
        let mut encoder = png::Encoder::new(&mut output, info.width, info.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|_| invalid())?;
        writer.write_image_data(&rgba).map_err(|_| invalid())?;
        writer.finish().map_err(|_| invalid())?;
    }
    Ok((info, output.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jpeg_upload_orients_pixels_and_strips_exif_without_allowing_native_footer() {
        let jpeg = include_bytes!("../../../tests/fixtures/owned-photo-orientation-6.jpg");
        let (info, output) = canonical_upload_png(jpeg).unwrap();
        assert_eq!((info.width, info.height), (640, 360));
        assert_eq!(canonical_png(&output).unwrap().1, output);
        assert!(!output.windows(4).any(|s| s == b"Exif"));
        assert!(!output.windows(5).any(|s| s == b"owned"));
        // Independently sample corners of the physically rotated decoded JPEG.
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(jpeg));
        let raw = decoder.decode().unwrap();
        assert_eq!(
            (
                decoder.info().unwrap().width,
                decoder.info().unwrap().height
            ),
            (360, 640)
        );
        let mut reader = png::Decoder::new(Cursor::new(&*output))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; 640 * 360 * 4];
        reader.next_frame(&mut pixels).unwrap();
        for (dest, source) in [
            (0, 639 * 360),
            (639, 0),
            (359 * 640, 640 * 360 - 1),
            (360 * 640 - 1, 359),
        ] {
            assert_eq!(
                &pixels[dest * 4..dest * 4 + 3],
                &raw[source * 3..source * 3 + 3]
            );
        }
        let mut appended = jpeg.to_vec();
        appended.push(0);
        assert!(canonical_upload_png(&appended).is_err());
        assert_eq!(canonical_received_png(&appended).unwrap().1, output);
        assert!(canonical_upload_png(&jpeg[..jpeg.len() - 2]).is_err());
        assert!(canonical_upload_png(b"not a JPEG").is_err());
    }
    #[test]
    fn received_jpeg_is_fully_decoded_bounded_and_rewritten() {
        let jpeg = include_bytes!("../../../tests/fixtures/owned-image.jpg");
        let (info, png) = canonical_received_png(jpeg).unwrap();
        assert_eq!(
            info,
            ImageInfo {
                width: 2,
                height: 2
            }
        );
        assert_eq!(canonical_png(&png).unwrap().1, png);
        assert!(canonical_received_png(&jpeg[..jpeg.len() - 1]).is_err());
        let mut appended = jpeg.to_vec();
        appended.extend_from_slice(jpeg);
        assert!(canonical_received_png(&appended).is_err());
        let mut native_footer = jpeg.to_vec();
        native_footer.extend_from_slice(b"owned opaque native footer");
        assert_eq!(canonical_received_png(&native_footer).unwrap().1, png);
        native_footer.resize(jpeg.len() + 65, 0);
        assert!(canonical_received_png(&native_footer).is_err());
        let mut large = jpeg.to_vec();
        let at = large.windows(2).position(|w| w == [255, 192]).unwrap();
        large[at + 5..at + 9].copy_from_slice(&[0x10, 0x01, 0x10, 0x01]);
        assert!(canonical_received_png(&large).is_err());
    }
    fn sample() -> Vec<u8> {
        let mut bytes = vec![];
        let mut e = png::Encoder::new(&mut bytes, 2, 2);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.add_text_chunk("private".into(), "owned metadata stripped".into())
            .unwrap();
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 255, 0])
            .unwrap();
        w.finish().unwrap();
        bytes
    }
    #[test]
    fn validates_pixels_removes_metadata_and_is_deterministic() {
        let (info, bytes) = canonical_png(&sample()).unwrap();
        assert_eq!(
            info,
            ImageInfo {
                width: 2,
                height: 2
            }
        );
        assert!(!bytes.windows(7).any(|s| s == b"private"));
        assert_eq!(canonical_png(&bytes).unwrap().1, bytes);
        let mut reader = png::Decoder::new(Cursor::new(&*bytes)).read_info().unwrap();
        let mut pixels = vec![0; 16];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(
            pixels,
            &[
                0, 0, 0, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255
            ]
        );
    }
    #[test]
    fn rejects_truncation_trailing_data_crc_damage_animation_and_size() {
        let bytes = sample();
        for n in [0, 7, 32, bytes.len() - 1] {
            assert!(canonical_png(&bytes[..n]).is_err());
        }
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(canonical_png(&bad).is_err());
        let mut bad = bytes.clone();
        bad[29] ^= 1;
        assert!(canonical_png(&bad).is_err());
        let mut bad = bytes.clone();
        bad[12..16].copy_from_slice(b"acTL");
        assert!(canonical_png(&bad).is_err());
        assert!(canonical_png(&vec![0; MAX_IMAGE_BYTES + 1]).is_err());
        let mut output = LimitedOutput(Zeroizing::new(vec![0; MAX_IMAGE_BYTES]));
        assert!(output.write_all(&[0]).is_err());
    }
}
