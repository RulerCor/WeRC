//! Strict single-frame Zstandard text decoding, with allocation and output budgets.
//! No dictionary, concatenated frame, trailing bytes, or lossy UTF-8 fallback.
use std::io::Read;
use zeroize::Zeroizing;

pub(crate) const MAX_INPUT: usize = 65536;
pub(crate) const MAX_TEXT: usize = 16384;
const MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
type Result<T> = std::result::Result<T, &'static str>;

// A streaming output limit alone does not bound the decoder's history allocation.
fn frame_budget(bytes: &[u8]) -> Result<Option<usize>> {
    let descriptor = *bytes.get(4).ok_or("content_invalid")?;
    if descriptor & 0x1b != 0 {
        return Err("compression_unsupported");
    }
    let single = descriptor & 0x20 != 0;
    let flag = descriptor >> 6;
    let mut offset = 5;
    if !single {
        let w = *bytes.get(offset).ok_or("content_invalid")?;
        offset += 1;
        let base = 1u64 << (10 + (w >> 3));
        if base + (base >> 3) * u64::from(w & 7) > 8 * 1024 * 1024 {
            return Err("content_oversized");
        }
    }
    let length = match flag {
        0 if single => 1,
        0 => 0,
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let encoded = bytes
        .get(offset..offset + length)
        .ok_or("content_invalid")?;
    if length == 0 {
        return Ok(None);
    }
    let mut size = 0u64;
    for (index, byte) in encoded.iter().enumerate() {
        size |= u64::from(*byte) << (8 * index);
    }
    if flag == 1 {
        size += 256;
    }
    if size > MAX_TEXT as u64 {
        return Err("content_oversized");
    }
    Ok(Some(size as usize))
}

pub(crate) fn decode(bytes: &[u8]) -> Result<(String, &'static str)> {
    if bytes.len() > MAX_INPUT {
        return Err("content_oversized");
    }
    if !bytes.starts_with(&MAGIC) {
        if bytes.len() > MAX_TEXT {
            return Err("content_oversized");
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "content_invalid")?;
        if text.contains('\0') {
            return Err("content_invalid");
        }
        return Ok((text.to_owned(), "text"));
    }
    let size = frame_budget(bytes)?;
    let mut decoder =
        ruzstd::decoding::StreamingDecoder::new(bytes).map_err(|_| "content_invalid")?;
    let mut decoded = Zeroizing::new(Vec::new());
    decoder
        .by_ref()
        .take(MAX_TEXT as u64 + 1)
        .read_to_end(&mut decoded)
        .map_err(|_| "content_invalid")?;
    if decoded.len() > MAX_TEXT {
        return Err("content_oversized");
    }
    if !decoder.decoder.is_finished()
        || !decoder.get_ref().is_empty()
        || size.is_some_and(|s| s != decoded.len())
    {
        return Err("content_invalid");
    }
    if let Some(checksum) = decoder.decoder.get_checksum_from_data()
        && Some(checksum) != decoder.decoder.get_calculated_checksum()
    {
        return Err("content_invalid");
    }
    let text = std::str::from_utf8(&decoded).map_err(|_| "content_invalid")?;
    if text.contains('\0') {
        return Err("content_invalid");
    }
    Ok((text.to_owned(), "text_zstd"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(bytes: &[u8]) -> Vec<u8> {
        assert!(bytes.len() < 256);
        let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, bytes.len() as u8];
        let block = ((bytes.len() as u32) << 3) | 1;
        frame.extend_from_slice(&block.to_le_bytes()[..3]);
        frame.extend_from_slice(bytes);
        frame
    }
    #[test]
    fn plain_and_independent_raw_frames_preserve_utf8() {
        assert_eq!(
            decode("自有文字 <img>".as_bytes()).unwrap(),
            ("自有文字 <img>".into(), "text")
        );
        assert_eq!(
            decode(&raw("压缩样本🙂".as_bytes())).unwrap(),
            ("压缩样本🙂".into(), "text_zstd")
        );
        assert_eq!(decode(&raw(b"")).unwrap().0, "");
    }
    #[test]
    fn validates_checksum_truncation_and_trailing_bytes() {
        let mut frame = raw(b"abc");
        frame[4] |= 4;
        // XXH64("abc", seed=0) low 32 bits, little endian.
        frame.extend_from_slice(&[0x99, 0x09, 0x77, 0xad]);
        assert_eq!(decode(&frame).unwrap().0, "abc");
        *frame.last_mut().unwrap() ^= 1;
        assert_eq!(decode(&frame).unwrap_err(), "content_invalid");
        let frame = raw(b"abc");
        for end in 4..frame.len() {
            assert!(decode(&frame[..end]).is_err());
        }
        let mut extra = frame.clone();
        extra.push(0);
        assert!(decode(&extra).is_err());
        extra = frame.clone();
        extra.extend_from_slice(&frame);
        assert!(decode(&extra).is_err());
        let mut wrong = frame;
        wrong[5] = 2;
        assert!(decode(&wrong).is_err());
    }
    #[test]
    fn bounds_history_output_and_rejects_invalid_text() {
        assert!(decode(&[0x28, 0xb5, 0x2f, 0xfd, 0, 0xf8]).is_err());
        assert!(
            decode(&[
                0x28, 0xb5, 0x2f, 0xfd, 0xe0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
            ])
            .is_err()
        );
        assert!(decode(&vec![b'a'; MAX_TEXT + 1]).is_err());
        assert!(decode(&vec![b'a'; MAX_INPUT + 1]).is_err());
        for bytes in [&b"a\0b"[..], &[0xff][..]] {
            assert!(decode(bytes).is_err());
            assert!(decode(&raw(bytes)).is_err());
        }
        // Unknown content size, 32KiB history, RLE output exceeds text budget.
        let mut bomb = vec![0x28, 0xb5, 0x2f, 0xfd, 0, 0x28];
        let block = (((MAX_TEXT + 1) as u32) << 3) | 3;
        bomb.extend_from_slice(&block.to_le_bytes()[..3]);
        bomb.push(b'x');
        assert_eq!(decode(&bomb).unwrap_err(), "content_oversized");
    }
}
