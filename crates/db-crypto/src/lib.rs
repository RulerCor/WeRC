use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use zeroize::Zeroizing;

pub const PAGE_SIZE: usize = 4096;
pub const RESERVED: usize = 80;

pub fn authentication_key(
    key: &[u8],
    salt: &[u8; 16],
) -> Result<Zeroizing<[u8; 32]>, &'static str> {
    if key.len() != 32 {
        return Err("KEY_FORMAT_INVALID");
    }
    let mut mac_salt = *salt;
    for byte in &mut mac_salt {
        *byte ^= 0x3a;
    }
    let mut output = Zeroizing::new([0; 32]);
    pbkdf2_hmac::<Sha512>(key, &mac_salt, 2, output.as_mut());
    Ok(output)
}

pub fn authenticate_page(mac_key: &[u8], number: u32, page: &[u8]) -> Result<(), &'static str> {
    if mac_key.len() != 32 || number == 0 || page.len() != PAGE_SIZE {
        return Err("DATABASE_PAGE_INVALID");
    }
    let start = if number == 1 { 16 } else { 0 };
    let end = PAGE_SIZE - RESERVED + 16;
    let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(mac_key).map_err(|_| "CRYPTO_FAILED")?;
    mac.update(&page[start..end]);
    mac.update(&number.to_le_bytes());
    mac.verify_slice(&page[end..])
        .map_err(|_| "KEY_OR_PAGE_AUTHENTICATION_FAILED")
}

// 仅认证固定格式的第一页，不解析SQLite、不保证其余页面、WAL或账号归属。
pub fn verify_first_page(key: &[u8], page: &[u8]) -> Result<(), &'static str> {
    if page.len() != PAGE_SIZE || page.starts_with(b"SQLite format 3\0") {
        return Err("DATABASE_FORMAT_UNSUPPORTED");
    }
    let salt = page[..16].try_into().map_err(|_| "DATABASE_PAGE_INVALID")?;
    let derived = authentication_key(key, salt)?;
    authenticate_page(derived.as_ref(), 1, page)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticates_salt_payload_and_little_endian_page_number() {
        let mut page = [0x35; PAGE_SIZE];
        let key = [0x5a; 32];
        let derived = authentication_key(&key, page[..16].try_into().unwrap()).unwrap();
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(derived.as_ref()).unwrap();
        mac.update(&page[16..4032]);
        mac.update(&1u32.to_le_bytes());
        page[4032..].copy_from_slice(&mac.finalize().into_bytes());
        assert!(verify_first_page(&key, &page).is_ok());
        assert!(verify_first_page(&[0x6b; 32], &page).is_err());
        assert!(authenticate_page(derived.as_ref(), 2, &page).is_err());
        page[300] ^= 1;
        assert!(verify_first_page(&key, &page).is_err());
        assert!(verify_first_page(&key, &page[..100]).is_err());
        assert!(authentication_key(&[0; 31], &[0; 16]).is_err());
    }
}
