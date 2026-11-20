//! Pure bounded decoder for an authenticated message's local GIF cache.
//! Format research reference: Zoeicx/Wechat-Emoticon-Parser v4.0-plus README.
use aes::{
    Aes128,
    cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray},
};
use md5::{Digest, Md5};
use zeroize::Zeroizing;
const LIMIT: usize = 1024 * 1024;

#[cfg(test)]
pub fn decode_with_seed(
    seed: u32,
    account: &str,
    bytes: &[u8],
    expected_md5: &str,
) -> Option<Zeroizing<Vec<u8>>> {
    decode_cache_with_seed(seed, account, bytes, expected_md5, false)
}
pub fn decode_cache_with_seed(
    seed: u32,
    account: &str,
    bytes: &[u8],
    expected_md5: &str,
    allow_wxgf: bool,
) -> Option<Zeroizing<Vec<u8>>> {
    if !(100_000_001..4_000_000_000).contains(&seed)
        || account.is_empty()
        || account.len() > 256
        || account.chars().any(char::is_control)
        || bytes.len() < 16
        || bytes.len() > LIMIT + 16
        || !bytes.len().is_multiple_of(16)
        || expected_md5.len() != 32
        || !expected_md5.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    let input = Zeroizing::new(format!("{seed}{account}EMOTICON"));
    let key = Zeroizing::new(<[u8; 16]>::from(Md5::digest(input.as_bytes())));
    let cipher = Aes128::new(GenericArray::from_slice(key.as_ref()));
    let mut header = GenericArray::clone_from_slice(&bytes[..16]);
    cipher.decrypt_block(&mut header);
    for (b, iv) in header.iter_mut().zip(key.iter()) {
        *b ^= *iv;
    }
    let wxgf = allow_wxgf && header.starts_with(b"wxgf");
    if !wxgf && !header.starts_with(b"GIF87a") && !header.starts_with(b"GIF89a") {
        return None;
    }
    let mut plain = Zeroizing::new(bytes.to_vec());
    let mut previous = Zeroizing::new(*key);
    for block in plain.as_chunks_mut::<16>().0 {
        let original = *block;
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
        for (b, iv) in block.iter_mut().zip(previous.iter()) {
            *b ^= *iv;
        }
        *previous = original;
    }
    let pad = usize::from(*plain.last()?);
    if pad == 0
        || pad > 16
        || plain.len() < pad
        || !plain[plain.len() - pad..]
            .iter()
            .all(|b| usize::from(*b) == pad)
    {
        return None;
    }
    let size = plain.len() - pad;
    plain.truncate(size);
    if wxgf {
        werc_domain::wxgf::parse(&plain).ok()?;
        return Some(plain);
    }
    if plain.len() > LIMIT
        || !hex::encode(Md5::digest(plain.as_slice())).eq_ignore_ascii_case(expected_md5)
    {
        return None;
    }
    Some(plain)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;
    const DATA: &[u8] = include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif");
    pub(crate) fn encoded(seed: u32, account: &str) -> Vec<u8> {
        encoded_bytes(seed, account, DATA)
    }
    fn encoded_bytes(seed: u32, account: &str, data: &[u8]) -> Vec<u8> {
        let key = <[u8; 16]>::from(Md5::digest(format!("{seed}{account}EMOTICON").as_bytes()));
        let cipher = Aes128::new(GenericArray::from_slice(&key));
        let mut bytes = data.to_vec();
        let pad = 16 - bytes.len() % 16;
        bytes.resize(bytes.len() + pad, pad as u8);
        let mut previous = key;
        for block in bytes.as_chunks_mut::<16>().0 {
            for (b, iv) in block.iter_mut().zip(previous.iter()) {
                *b ^= *iv;
            }
            cipher.encrypt_block(GenericArray::from_mut_slice(block));
            previous.copy_from_slice(block);
        }
        bytes
    }
    #[test]
    fn converted_cache_requires_explicit_profile_and_body_checksum() {
        let data = include_bytes!("../../../tests/fixtures/owned-received-emoticon.wxgf");
        let cipher = encoded_bytes(1234567890, "owned-account", data);
        let original = "ab".repeat(16);
        assert!(decode_with_seed(1234567890, "owned-account", &cipher, &original).is_none());
        assert_eq!(
            decode_cache_with_seed(1234567890, "owned-account", &cipher, &original, true)
                .unwrap()
                .as_slice(),
            data
        );
        assert!(
            decode_cache_with_seed(1234567891, "owned-account", &cipher, &original, true).is_none()
        );
        assert!(
            decode_cache_with_seed(1234567890, "other-account", &cipher, &original, true).is_none()
        );
        let mut bad = data.to_vec();
        bad[100] ^= 1;
        let cipher = encoded_bytes(1234567890, "owned-account", &bad);
        assert!(
            decode_cache_with_seed(1234567890, "owned-account", &cipher, &original, true).is_none()
        );
    }
    #[test]
    fn restores_complete_owned_gif_and_binds_identity_and_md5() {
        let bytes = encoded(1234567890, "owned-account");
        let hash = hex::encode(Md5::digest(DATA));
        assert_eq!(
            decode_with_seed(1234567890, "owned-account", &bytes, &hash)
                .unwrap()
                .as_slice(),
            DATA
        );
        assert!(decode_with_seed(1234567891, "owned-account", &bytes, &hash).is_none());
        assert!(decode_with_seed(1234567890, "other-account", &bytes, &hash).is_none());
        assert!(decode_with_seed(1234567890, "owned-account", &bytes, &"a".repeat(32)).is_none());
    }
    #[test]
    fn rejects_corruption_truncation_and_limits() {
        let hash = hex::encode(Md5::digest(DATA));
        let mut bytes = encoded(1234567890, "owned-account");
        bytes[32] ^= 1;
        assert!(decode_with_seed(1234567890, "owned-account", &bytes, &hash).is_none());
        let bytes = encoded(1234567890, "owned-account");
        assert!(
            decode_with_seed(
                1234567890,
                "owned-account",
                &bytes[..bytes.len() - 1],
                &hash
            )
            .is_none()
        );
        let mut bad = bytes;
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(decode_with_seed(1234567890, "owned-account", &bad, &hash).is_none());
        assert!(
            decode_with_seed(1234567890, "owned-account", &vec![0; LIMIT + 32], &hash).is_none()
        );
    }
}
