use aes::Aes256;
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::NoPadding};
use werc_db_crypto::{authenticate_page, authentication_key};
use zeroize::Zeroizing;

pub use werc_db_crypto::{PAGE_SIZE, RESERVED};
pub type ReadResult<T> = Result<T, &'static str>;
pub(crate) fn sqlite_error(error: rusqlite::Error, fallback: &'static str) -> &'static str {
    if error.sqlite_error_code() == Some(rusqlite::ErrorCode::OutOfMemory) {
        "DATABASE_LIMIT_EXCEEDED"
    } else {
        fallback
    }
}

pub(crate) fn check_database(connection: &rusqlite::Connection) -> ReadResult<()> {
    let result: Result<String, _> = connection.query_row("PRAGMA quick_check(1)", [], |r| r.get(0));
    match result {
        Ok(check) if check == "ok" => Ok(()),
        Ok(_) => Err("DATABASE_CORRUPT"),
        Err(rusqlite::Error::SqliteFailure(_, Some(ref message)))
            if message.contains("no such tokenizer:") || message.contains("no such module:") =>
        {
            // WCDB may use extensions that bundled SQLite does not provide.
            // Do not load arbitrary modules or claim that their data was read.
            Err("SCHEMA_EXTENSION_UNSUPPORTED")
        }
        Err(error) => Err(sqlite_error(error, "DATABASE_CORRUPT")),
    }
}

// 有意只实现一种公开的磁盘格式，不冒充完整SQLCipher引擎，也不猜测参数。
pub struct PageDecoder {
    key: Zeroizing<Vec<u8>>,
    authentication_key: Zeroizing<[u8; 32]>,
    salt: [u8; 16],
}
impl PageDecoder {
    pub fn new(key_hex: &str, first: &[u8]) -> ReadResult<Self> {
        if key_hex.len() != 64 {
            return Err("KEY_FORMAT_INVALID");
        }
        let key = Zeroizing::new(hex::decode(key_hex).map_err(|_| "KEY_FORMAT_INVALID")?);
        if key.len() != 32 || first.len() < PAGE_SIZE {
            return Err("DATABASE_FORMAT_UNSUPPORTED");
        }
        let salt: [u8; 16] = first[..16]
            .try_into()
            .map_err(|_| "DATABASE_FORMAT_UNSUPPORTED")?;
        let authentication_key = authentication_key(&key, &salt)?;
        Ok(Self {
            key,
            authentication_key,
            salt,
        })
    }
    pub fn page(&self, number: u32, bytes: &[u8]) -> ReadResult<Zeroizing<Vec<u8>>> {
        if number == 0 || bytes.len() != PAGE_SIZE {
            return Err("DATABASE_PAGE_INVALID");
        }
        let mut output = Zeroizing::new(vec![0_u8; PAGE_SIZE]);
        self.page_into(number, bytes, &mut output)?;
        Ok(output)
    }
    pub fn page_into(&self, number: u32, bytes: &[u8], output: &mut [u8]) -> ReadResult<()> {
        if number == 0 || bytes.len() != PAGE_SIZE || output.len() != PAGE_SIZE {
            return Err("DATABASE_PAGE_INVALID");
        }
        let start = if number == 1 { 16 } else { 0 };
        if number == 1 && bytes[..16] != self.salt {
            return Err("DATABASE_SALT_CHANGED");
        }
        let payload_end = PAGE_SIZE - RESERVED;
        let iv_end = payload_end + 16;
        authenticate_page(self.authentication_key.as_ref(), number, bytes)?;
        output.fill(0);
        output[start..payload_end].copy_from_slice(&bytes[start..payload_end]);
        cbc::Decryptor::<Aes256>::new_from_slices(&self.key, &bytes[payload_end..iv_end])
            .map_err(|_| "CRYPTO_FAILED")?
            .decrypt_padded_mut::<NoPadding>(&mut output[start..payload_end])
            .map_err(|_| "CRYPTO_FAILED")?;
        if number == 1 {
            output[..16].copy_from_slice(b"SQLite format 3\0");
            if output[16..18] != [0x10, 0] || output[20] != RESERVED as u8 {
                return Err("CIPHER_PARAMETERS_UNSUPPORTED");
            }
        }
        Ok(())
    }
}

// Constant-space pass. Every complete current-generation frame, including the
// uncommitted tail, is checksum-checked and authenticated. Older generations
// terminate the log, as in SQLite WAL reuse. Only committed pages are replayed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WalInfo {
    pub committed: usize,
    pub pages: usize,
}
pub(crate) fn scan_wal(
    file: &mut std::fs::File,
    length: u64,
    decoder: Option<&PageDecoder>,
    mut visit: impl FnMut(usize, u32, &[u8]) -> ReadResult<()>,
) -> ReadResult<WalInfo> {
    use std::io::{Read, Seek, SeekFrom};
    if length == 0 {
        return Ok(WalInfo::default());
    }
    let stride = 24 + PAGE_SIZE;
    if length < 32 {
        return Err("WAL_TRUNCATED");
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "SOURCE_READ_FAILED")?;
    let mut header = [0u8; 32];
    file.read_exact(&mut header).map_err(|_| "WAL_TRUNCATED")?;
    let magic = word(&header[..4]);
    if !matches!(magic, 0x377f0682 | 0x377f0683)
        || word(&header[4..8]) != 3007000
        || word(&header[8..12]) != PAGE_SIZE as u32
    {
        return Err("WAL_FORMAT_UNSUPPORTED");
    }
    let little = magic == 0x377f0682;
    let mut sum = checksum(&header[..24], (0, 0), little);
    if sum != (word(&header[24..28]), word(&header[28..32])) {
        return Err("WAL_CHECKSUM_INVALID");
    }
    let mut info = WalInfo::default();
    let mut frame = Zeroizing::new(vec![0u8; stride]);
    let mut decoded = Zeroizing::new([0u8; PAGE_SIZE]);
    for index in 1..=((length - 32) / stride as u64) as usize {
        file.read_exact(&mut frame).map_err(|_| "WAL_TRUNCATED")?;
        if frame[8..16] != header[16..24] {
            // SQLite reuses WAL files without truncating their old allocation.
            // A different generation ends the current log; never replay or
            // authenticate stale pages using the current database key.
            return Ok(info);
        }
        sum = checksum(&frame[..8], sum, little);
        sum = checksum(&frame[24..], sum, little);
        if sum != (word(&frame[16..20]), word(&frame[20..24])) {
            return Err("WAL_CHECKSUM_INVALID");
        }
        let page = word(&frame[..4]);
        let size = word(&frame[4..8]);
        let maximum = crate::snapshot::MAX_DB_BYTES / PAGE_SIZE as u64;
        if page == 0 || page as u64 > maximum || size as u64 > maximum {
            return Err("DATABASE_LIMIT_EXCEEDED");
        }
        if let Some(decoder) = decoder {
            decoder.page_into(page, &frame[24..], decoded.as_mut())?;
        } else {
            decoded.copy_from_slice(&frame[24..]);
        }
        visit(index, page, decoded.as_ref())?;
        if size != 0 {
            info = WalInfo {
                committed: index,
                pages: size as usize,
            };
        }
    }
    if !(length - 32).is_multiple_of(stride as u64) {
        return Err("WAL_TRUNCATED");
    }
    Ok(info)
}

pub fn decode_database(
    bytes: &[u8],
    decoder: Option<&PageDecoder>,
) -> ReadResult<Zeroizing<Vec<u8>>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(PAGE_SIZE) {
        return Err("DATABASE_FORMAT_UNSUPPORTED");
    }
    let Some(decoder) = decoder else {
        return Ok(Zeroizing::new(bytes.to_vec()));
    };
    let mut plaintext = Zeroizing::new(Vec::with_capacity(bytes.len()));
    for (index, chunk) in bytes.as_chunks::<PAGE_SIZE>().0.iter().enumerate() {
        plaintext.extend_from_slice(&decoder.page((index + 1) as u32, chunk)?);
    }
    Ok(plaintext)
}

fn word(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().expect("fixed word"))
}
fn checksum(bytes: &[u8], mut s: (u32, u32), little: bool) -> (u32, u32) {
    for chunk in bytes.as_chunks::<8>().0 {
        let a: [u8; 4] = chunk[..4].try_into().expect("fixed word");
        let b: [u8; 4] = chunk[4..].try_into().expect("fixed word");
        let a = if little {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        };
        let b = if little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        };
        s.0 = s.0.wrapping_add(a).wrapping_add(s.1);
        s.1 = s.1.wrapping_add(b).wrapping_add(s.0);
    }
    s
}

// 只从已提交且校验完整的WAL构造内存视图，不checkpoint、不修改源文件。
pub fn apply_wal(
    database: &mut Zeroizing<Vec<u8>>,
    wal: &[u8],
    decoder: Option<&PageDecoder>,
) -> ReadResult<usize> {
    if wal.is_empty() {
        return Ok(0);
    }
    if wal.len() < 32 {
        return Err("WAL_TRUNCATED");
    }
    let magic = word(&wal[..4]);
    if !matches!(magic, 0x377f0682 | 0x377f0683)
        || word(&wal[4..8]) != 3007000
        || word(&wal[8..12]) != PAGE_SIZE as u32
    {
        return Err("WAL_FORMAT_UNSUPPORTED");
    }
    let little = magic == 0x377f0682;
    let mut sum = checksum(&wal[..24], (0, 0), little);
    if sum != (word(&wal[24..28]), word(&wal[28..32])) {
        return Err("WAL_CHECKSUM_INVALID");
    }
    let stride = 24 + PAGE_SIZE;
    let mut stale_tail = false;
    let mut frames = Vec::new();
    let mut last_commit = None;
    for frame in wal[32..].chunks_exact(stride) {
        if frame[8..16] != wal[16..24] {
            stale_tail = true;
            break;
        }
        sum = checksum(&frame[..8], sum, little);
        sum = checksum(&frame[24..], sum, little);
        if sum != (word(&frame[16..20]), word(&frame[20..24])) {
            return Err("WAL_CHECKSUM_INVALID");
        }
        let page = word(&frame[..4]);
        let size = word(&frame[4..8]);
        if page == 0 || page > 16384 || size > 16384 {
            return Err("DATABASE_LIMIT_EXCEEDED");
        }
        // 包括尚未提交的完整帧也做认证；不把损坏尾部悄悄当作成功。
        let plaintext = if let Some(d) = decoder {
            d.page(page, &frame[24..])?
        } else {
            Zeroizing::new(frame[24..].to_vec())
        };
        frames.push((page, plaintext));
        if size != 0 {
            last_commit = Some((frames.len(), size));
        }
    }
    if !stale_tail && !(wal.len() - 32).is_multiple_of(stride) {
        return Err("WAL_TRUNCATED");
    }
    let Some((committed, pages)) = last_commit else {
        return Ok(0);
    };
    database.resize(pages as usize * PAGE_SIZE, 0);
    for (page, contents) in frames.iter().take(committed) {
        if *page <= pages {
            let offset = (*page as usize - 1) * PAGE_SIZE;
            database[offset..offset + PAGE_SIZE].copy_from_slice(contents);
        }
    }
    Ok(committed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_geometry_and_headers() {
        assert!(decode_database(&[0; 7], None).is_err());
        assert!(PageDecoder::new("bad", &[0; PAGE_SIZE]).is_err());
        assert!(apply_wal(&mut Zeroizing::new(vec![0; PAGE_SIZE]), &[0; 32], None).is_err());
    }
    #[test]
    fn rejects_unauthenticated_pages() {
        let decoder = PageDecoder::new(&"5a".repeat(32), &[0; PAGE_SIZE]).unwrap();
        assert!(decoder.page(1, &[0; PAGE_SIZE]).is_err());
        assert!(decoder.page(0, &[0; PAGE_SIZE]).is_err());
    }
}
