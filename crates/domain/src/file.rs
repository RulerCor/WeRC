//! Ordinary attachments: bounded bytes and a portable basename, never a path.
use crate::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub fn validate_file_name(name: &str) -> AppResult<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 120
        || name.trim() != name
        || name.ends_with('.')
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "文件名须为1–120个UTF-8字节的普通文件名，不能包含路径、控制字符或设备名",
        ));
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileAsset {
    pub id: String,
    pub account_id: String,
    pub sha256: String,
    pub size: usize,
    pub created_at: String,
    pub expires_at: String,
}
pub struct ProtectedFile {
    pub asset: FileAsset,
    pub wrapped_key: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}
#[derive(Serialize)]
pub struct FilePreview {
    pub asset: FileAsset,
    pub file_name: String,
}
pub struct FileContents {
    pub file_name: String,
    pub bytes: Zeroizing<Vec<u8>>,
}
impl Drop for FileContents {
    fn drop(&mut self) {
        self.file_name.zeroize();
    }
}
pub fn pack_file(name: &str, bytes: &[u8]) -> AppResult<Zeroizing<Vec<u8>>> {
    validate_file_name(name)?;
    if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
        return Err(AppError::new("QUOTA_EXCEEDED", "文件须非空且最多8MiB"));
    }
    let mut packed = Zeroizing::new(Vec::with_capacity(2 + name.len() + bytes.len()));
    packed.extend_from_slice(&(name.len() as u16).to_le_bytes());
    packed.extend_from_slice(name.as_bytes());
    packed.extend_from_slice(bytes);
    Ok(packed)
}
pub fn unpack_file(packed: &[u8]) -> AppResult<FileContents> {
    let invalid = || AppError::new("ASSET_CORRUPTED", "文件保护内容无效");
    if packed.len() < 4 {
        return Err(invalid());
    }
    let length = u16::from_le_bytes([packed[0], packed[1]]) as usize;
    if length == 0
        || length > 120
        || packed.len() <= 2 + length
        || packed.len() - 2 - length > MAX_FILE_BYTES
    {
        return Err(invalid());
    }
    let name = std::str::from_utf8(&packed[2..2 + length]).map_err(|_| invalid())?;
    validate_file_name(name).map_err(|_| invalid())?;
    Ok(FileContents {
        file_name: name.into(),
        bytes: Zeroizing::new(packed[2 + length..].to_vec()),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_file_roundtrip_preserves_utf8_name_and_exact_bytes() {
        let p = pack_file("自动化测试.txt", b"owned-file\n").unwrap();
        let f = unpack_file(&p).unwrap();
        assert_eq!(f.file_name, "自动化测试.txt");
        assert_eq!(&*f.bytes, b"owned-file\n");
        assert!(pack_file("empty.txt", b"").is_err());
        assert!(unpack_file(&[255, 255, 0, 0]).is_err());
    }
    #[test]
    fn filenames_are_basenames_not_paths_or_device_aliases() {
        for name in [
            "", ".", "..", "a/hi", "a\\hi", "C:hi", "CON.txt", "com1", "LPT9.txt", "a\n.txt", "a.",
            "a ",
        ] {
            assert!(validate_file_name(name).is_err(), "{name:?}");
        }
        assert!(validate_file_name("safe-文件.txt").is_ok());
        assert!(validate_file_name(&"a".repeat(121)).is_err());
    }
}
