use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn check(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(AppError::new("QUERY_CANCELED", "表情读取已取消"))
    } else {
        Ok(())
    }
}

// Reuse the read-only scanner and process identity checks. A candidate is never
// returned or retained. GIFs bind the original digest; type8 converted caches
// instead require the explicitly selected wxgf profile and whole-body checksum.
pub(super) fn scan_cache<M: Memory>(
    memory: &mut M,
    regions: &[Region],
    account: &str,
    cache: &[u8],
    md5: &str,
    allow_wxgf: bool,
    cancel: &AtomicBool,
) -> AppResult<Zeroizing<Vec<u8>>> {
    check(cancel)?;
    if regions.len() > 65536 {
        return Err(limit());
    }
    let mut scan = Scan {
        memory,
        remaining: 256 * 1024 * 1024,
        deadline: Instant::now() + Duration::from_secs(20),
        incomplete: false,
    };
    let mut tested = Zeroizing::new(Vec::<u32>::new());
    for region in regions {
        let mut offset = 0;
        while offset < region.size {
            check(cancel)?;
            let length = CHUNK.min(region.size - offset);
            if let Some(bytes) = scan.read(region.base + offset as u64, length)? {
                let mut at = 0;
                while at < bytes.len() {
                    if !bytes[at].is_ascii_digit() {
                        at += 1;
                        continue;
                    }
                    let begin = at;
                    while at < bytes.len() && bytes[at].is_ascii_digit() {
                        at += 1;
                    }
                    // Do not interpret a suffix/prefix at a chunk boundary.
                    if (begin == 0 && offset > 0)
                        || (at == bytes.len() && offset + length < region.size)
                        || !(8..=12).contains(&(at - begin))
                    {
                        continue;
                    }
                    let seed = bytes[begin..at].iter().try_fold(0u32, |n, b| {
                        n.checked_mul(10)?.checked_add(u32::from(b - b'0'))
                    });
                    let Some(seed) = seed.filter(|s| (100_000_001..4_000_000_000).contains(s))
                    else {
                        continue;
                    };
                    if tested.contains(&seed) {
                        continue;
                    }
                    if tested.len() == 4096 {
                        return Err(limit());
                    }
                    tested.push(seed);
                    check(cancel)?;
                    if Instant::now() >= scan.deadline {
                        return Err(limit());
                    }
                    if let Some(bytes) = crate::emoticon_cache::decode_cache_with_seed(
                        seed, account, cache, md5, allow_wxgf,
                    ) {
                        if !bytes.starts_with(b"wxgf") {
                            werc_domain::emoticon::inspect_gif("received-emoticon.gif", &bytes)?;
                        }
                        check(cancel)?;
                        return Ok(bytes);
                    }
                }
            }
            if offset + length == region.size {
                break;
            }
            offset += length.saturating_sub(128);
        }
    }
    Err(AppError::new(
        "EMOTICON_CACHE_UNAVAILABLE",
        "未找到与当前账号及消息匹配的表情缓存解码材料",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Owned(Vec<u8>);
    impl Memory for Owned {
        fn read(&mut self, address: u64, size: usize) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
            Ok(self
                .0
                .get((address - 0x10000) as usize..(address - 0x10000) as usize + size)
                .map(|b| Zeroizing::new(b.to_vec())))
        }
    }
    #[test]
    fn overlapping_chunks_find_complete_candidate_without_persisting_material() {
        let mut data = vec![0; CHUNK + 80];
        data[CHUNK - 5..CHUNK + 5].copy_from_slice(b"1234567890");
        let regions = [Region {
            base: 0x10000,
            size: data.len(),
        }];
        let cipher = crate::emoticon_cache::tests::encoded(1234567890, "owned-account");
        let bytes = scan_cache(
            &mut Owned(data),
            &regions,
            "owned-account",
            &cipher,
            "df4813390984d6155aabb73ea22dade0",
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            bytes.as_slice(),
            include_bytes!("../../../../tests/fixtures/owned-send-emoticon.gif")
        );
    }
    #[test]
    fn cancel_wrong_account_and_longer_digit_runs_never_produce_media() {
        let cipher = crate::emoticon_cache::tests::encoded(1234567890, "owned-account");
        for (data, account, canceled) in [
            (b" 1234567890 ".as_slice(), "owned-account", true),
            (b" 1234567890 ", "wrong-account", false),
            (b"9991234567890999", "owned-account", false),
        ] {
            let regions = [Region {
                base: 0x10000,
                size: data.len(),
            }];
            assert!(
                scan_cache(
                    &mut Owned(data.to_vec()),
                    &regions,
                    account,
                    &cipher,
                    "df4813390984d6155aabb73ea22dade0",
                    false,
                    &AtomicBool::new(canceled)
                )
                .is_err()
            );
        }
    }
    #[test]
    fn candidate_budget_is_enforced() {
        let data = (100_000_001..100_004_099u32)
            .map(|n| format!(" {n} "))
            .collect::<String>()
            .into_bytes();
        let regions = [Region {
            base: 0x10000,
            size: data.len(),
        }];
        let cipher = crate::emoticon_cache::tests::encoded(1234567890, "owned-account");
        assert_eq!(
            scan_cache(
                &mut Owned(data),
                &regions,
                "owned-account",
                &cipher,
                "df4813390984d6155aabb73ea22dade0",
                false,
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .code,
            "KEY_SCAN_LIMIT"
        );
    }
}
