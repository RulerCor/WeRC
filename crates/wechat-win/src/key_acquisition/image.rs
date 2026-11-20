use super::*;
use std::sync::atomic::AtomicBool;
use werc_domain::connection::ImageCacheKey;
pub(super) fn scan_image<M: Memory>(
    memory: &mut M,
    regions: &[Region],
    sample: &[u8],
    cancel: &AtomicBool,
) -> AppResult<ImageCacheKey> {
    crate::image_cache::check(cancel)?;
    let empty = ImageCacheKey {
        aes: None,
        xor: None,
    };
    if !sample.starts_with(b"\x07\x08V2\x08\x07") {
        let (_, xor) = crate::image_cache::decode_bytes(sample, &empty)?;
        let result = ImageCacheKey { aes: None, xor };
        crate::image_cache::decode(sample, &result, cancel)?;
        return Ok(result);
    }
    let mut scan = Scan {
        memory,
        remaining: 2 * 1024 * 1024 * 1024,
        deadline: Instant::now() + Duration::from_secs(45),
        incomplete: false,
    };
    let mut found = None;
    let mut tested = 0usize;
    let mut seen = BTreeSet::new();
    // Hexadecimal material is common; check it before other ASCII keys so
    // ordinary application strings cannot consume the candidate budget first.
    for hex_only in [true, false] {
        scan.chunks(regions, |_, _, bytes| {
            crate::image_cache::check(cancel)?;
            let mut offset = 0;
            while offset < bytes.len() {
                if !(if hex_only {
                    bytes[offset].is_ascii_hexdigit()
                } else {
                    bytes[offset].is_ascii_alphanumeric()
                }) {
                    offset += 1;
                    continue;
                }
                let start = offset;
                while offset < bytes.len()
                    && (if hex_only {
                        bytes[offset].is_ascii_hexdigit()
                    } else {
                        bytes[offset].is_ascii_alphanumeric()
                    })
                {
                    offset += 1;
                }
                let run = &bytes[start..offset];
                if !(16..=64).contains(&run.len()) {
                    continue;
                }
                for candidate in run.windows(16) {
                    use sha2::Digest;
                    let key = Zeroizing::new(
                        <[u8; 16]>::try_from(candidate).map_err(|_| AppError::internal())?,
                    );
                    if !seen.insert(sha2::Sha256::digest(&*key).to_vec()) {
                        continue;
                    }
                    tested += 1;
                    if tested > 100_000 {
                        return Err(limit());
                    }
                    if !crate::image_cache::first_block_matches(sample, &key) {
                        continue;
                    }
                    let tentative = ImageCacheKey {
                        aes: Some(Zeroizing::new(*key)),
                        xor: None,
                    };
                    if let Ok((_, xor)) = crate::image_cache::decode_bytes(sample, &tentative) {
                        let result = ImageCacheKey {
                            aes: Some(Zeroizing::new(*key)),
                            xor,
                        };
                        match crate::image_cache::decode(sample, &result, cancel) {
                            Ok(_) => {
                                found = Some(result);
                                return Ok(true);
                            }
                            Err(error) if error.code == "QUERY_CANCELED" => return Err(error),
                            Err(_) => {}
                        }
                    }
                }
            }
            Ok(false)
        })?;
        if found.is_some() {
            break;
        }
    }
    // Candidate material is only transient; no memory dumps or logs are kept.
    found.ok_or_else(|| {
        AppError::new(
            "IMAGE_KEY_NOT_FOUND",
            "当前内存未找到能完整解码样本的图片密钥；可选择最近打开的完整JPEG/PNG缓存或手工填写",
        )
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Owned {
        bytes: Vec<u8>,
    }
    impl Memory for Owned {
        fn read(&mut self, address: u64, size: usize) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
            let start = (address - 0x10000) as usize;
            Ok(self
                .bytes
                .get(start..start + size)
                .map(|b| Zeroizing::new(b.to_vec())))
        }
    }
    #[test]
    fn image_scan_requires_full_sample_and_finds_ascii_key_without_writes() {
        let sample = crate::image_cache::tests::encrypted_fixture();
        let mut memory = Owned {
            bytes: b"owned fixture, wrong key ffffffffffffffff;qZ7mN0pL2xT5vB9r;end".to_vec(),
        };
        let region = Region {
            base: 0x10000,
            size: memory.bytes.len(),
        };
        let before = memory.bytes.clone();
        let key = scan_image(&mut memory, &[region], &sample, &AtomicBool::new(false)).unwrap();
        assert_eq!(key.aes.as_deref(), Some(&crate::image_cache::tests::KEY));
        assert_eq!(key.xor, Some(0xa5));
        assert_eq!(memory.bytes, before);
        let region = Region {
            base: 0x10000,
            size: memory.bytes.len(),
        };
        assert_eq!(
            scan_image(&mut memory, &[region], &sample, &AtomicBool::new(true))
                .err()
                .unwrap()
                .code,
            "QUERY_CANCELED"
        );
    }
}
