mod emoticon;
mod windows;
mod image;
#[cfg(windows)]
pub(crate) use windows::acquire_image;

#[cfg(windows)]
pub(crate) use windows::read_emoticon;

use memchr::memmem;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use werc_domain::{AppError, AppResult};
use zeroize::Zeroizing;

// 布局研究：TANGandXUE/wcdb-key-tool@79f1b5b，MIT，许可见licenses。
// 仅移植只读Config.Cipher路径；不附加调试器、不注入、不修改权限或目标内存。
/// The Config.Cipher scanner has no version-specific code addresses. Candidate
/// keys are accepted only after the selected SQLCipher page authenticates.
pub use werc_domain::connection::supports_config_cipher;

pub(crate) const NEEDLE: &[u8] = b"com.Tencent.WCDB.Config.Cipher";
pub(crate) const MASK: [u8; 32] = [
    0xd2, 0xc7, 0x44, 0x24, 0x58, 0x02, 0, 0, 0, 0x48, 0x89, 0x44, 0x24, 0x50, 0x48, 0x8b, 0x45, 0,
    0x48, 0x84, 0x4c, 0x24, 0x48, 0x48, 0x89, 0x44, 0x25, 0x40, 0x48, 0x58, 0x4c, 0x24,
];
const CHUNK: usize = 1024 * 1024;
const MAX_ADDRESS: u64 = 0x0000_8000_0000_0000;

#[derive(Clone, Copy)]
struct Region {
    base: u64,
    size: usize,
}
trait Memory {
    fn read(&mut self, address: u64, size: usize) -> AppResult<Option<Zeroizing<Vec<u8>>>>;
}
struct ControlledMemory<'a, 'b, M> {
    memory: &'a mut M,
    control: &'a werc_application::ports::KeyScanControl<'b>,
    bytes: u64,
}
impl<M: Memory> Memory for ControlledMemory<'_, '_, M> {
    fn read(&mut self, address: u64, size: usize) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
        self.control.check()?;
        let result = self.memory.read(address, size)?;
        self.control.check()?;
        self.bytes = self.bytes.saturating_add(result.as_ref().map_or(0, |b| b.len() as u64));
        (self.control.progress)(self.bytes);
        Ok(result)
    }
}
struct Scan<'a, M> {
    memory: &'a mut M,
    remaining: usize,
    deadline: Instant,
    incomplete: bool,
}
fn limit() -> AppError {
    AppError::new(
        "KEY_SCAN_LIMIT",
        "取钥已达到时间、字节或候选数量上限；未保存未验真的候选",
    )
}
impl<M: Memory> Scan<'_, M> {
    fn read(&mut self, address: u64, size: usize) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
        if Instant::now() >= self.deadline || size > self.remaining {
            return Err(limit());
        }
        if address < 0x10000
            || address
                .checked_add(size as u64)
                .is_none_or(|end| end > MAX_ADDRESS)
        {
            return Ok(None);
        }
        self.remaining -= size;
        let bytes = self.memory.read(address, size)?;
        if bytes.as_ref().is_none_or(|b| b.len() != size) {
            self.incomplete = true;
            return Ok(None);
        }
        Ok(bytes)
    }
    fn chunks(
        &mut self,
        regions: &[Region],
        mut visit: impl FnMut(&mut Self, u64, &[u8]) -> AppResult<bool>,
    ) -> AppResult<bool> {
        for region in regions {
            let mut offset = 0;
            while offset < region.size {
                let length = CHUNK.min(region.size - offset);
                if let Some(bytes) = self.read(region.base + offset as u64, length)?
                    && visit(self, region.base + offset as u64, &bytes)?
                {
                    return Ok(true);
                }
                if offset + length == region.size {
                    break;
                }
                offset += length.saturating_sub(128);
            }
        }
        Ok(false)
    }
}
fn word(bytes: &[u8], offset: usize) -> u64 {
    bytes
        .get(offset..offset + 8)
        .and_then(|v| v.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0)
}
fn key_from_blob(
    blob: &[u8],
    first_page: &[u8],
    tested: &mut usize,
) -> AppResult<Option<Zeroizing<[u8; 32]>>> {
    if blob.is_empty() || blob.len() > 1024 {
        return Ok(None);
    }
    let decoded = Zeroizing::new(
        blob.iter()
            .enumerate()
            .map(|(i, b)| b ^ MASK[i % MASK.len()])
            .collect::<Vec<_>>(),
    );
    for begin in 0..decoded.len().saturating_sub(2) {
        if !matches!(decoded[begin], b'x' | b'X') || decoded[begin + 1] != b'\'' {
            continue;
        }
        let rest = &decoded[begin + 2..];
        let count = rest.iter().take_while(|b| b.is_ascii_hexdigit()).count();
        if !(64..=192).contains(&count) || rest.get(count) != Some(&b'\'') {
            continue;
        }
        let mut starts = vec![0];
        if count > 96 {
            starts.extend((0..=count - 64).step_by(32));
            starts.push(count - 64);
        }
        starts.sort_unstable();
        starts.dedup();
        for start in starts {
            *tested += 1;
            if *tested > 2048 {
                return Err(limit());
            }
            let mut key = Zeroizing::new([0u8; 32]);
            if hex::decode_to_slice(&rest[start..start + 64], key.as_mut()).is_ok()
                && werc_db_crypto::verify_first_page(key.as_ref(), first_page).is_ok()
            {
                return Ok(Some(key));
            }
        }
    }
    Ok(None)
}
fn scan<M: Memory>(
    memory: &mut M,
    regions: &[Region],
    first_page: &[u8],
) -> AppResult<Zeroizing<[u8; 32]>> {
    if regions.len() > 65536 {
        return Err(limit());
    }
    let mut scan = Scan {
        memory,
        remaining: 2 * 1024 * 1024 * 1024,
        deadline: Instant::now() + Duration::from_secs(30),
        incomplete: false,
    };
    let mut names = BTreeSet::new();
    scan.chunks(regions, |_, base, bytes| {
        for offset in memmem::find_iter(bytes, NEEDLE) {
            names.insert(base + offset as u64);
            if names.len() > 128 {
                return Err(limit());
            }
        }
        Ok(false)
    })?;
    let mut key = None;
    let mut nodes = BTreeSet::new();
    let mut tested = 0;
    let length = (NEEDLE.len() as u64).to_le_bytes();
    if !names.is_empty() {
        scan.chunks(regions, |scan, base, bytes| {
            for offset in memmem::find_iter(bytes, &length) {
                if offset < 8 || !names.contains(&word(bytes, offset - 8)) {
                    continue;
                }
                let Some(node_base) = (base + offset as u64).checked_sub(0x18) else {
                    continue;
                };
                if !nodes.insert(node_base) {
                    continue;
                }
                if nodes.len() > 4096 {
                    return Err(limit());
                }
                let Some(node) = scan.read(node_base, 0x50)? else {
                    continue;
                };
                if !names.contains(&word(&node, 0x10)) || word(&node, 0x18) != NEEDLE.len() as u64 {
                    continue;
                }
                let Some(object) = word(&node, 0x28).checked_add(0x88) else {
                    continue;
                };
                let Some(object) = scan.read(object, 0x28)? else {
                    continue;
                };
                let length = word(&object, 0x10);
                if length == 0 || length > 1024 {
                    continue;
                }
                let Some(blob) = scan.read(word(&object, 8), length as usize)? else {
                    continue;
                };
                key = key_from_blob(&blob, first_page, &mut tested)?;
                if key.is_some() {
                    return Ok(true);
                }
            }
            Ok(false)
        })?;
    }
    key.ok_or_else(|| {
        AppError::new(
            if scan.incomplete {
                "KEY_SCAN_INCOMPLETE"
            } else {
                "KEY_NOT_FOUND"
            },
            "没有找到与所选数据库第一页认证匹配的密钥；不会保存候选或尝试写入",
        )
    })
}

pub fn acquire(
    installation: &werc_domain::connection::ClientInstallation,
    pid: u32,
    database: &str,
) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
    windows::acquire(installation, pid, database)
}
pub fn acquire_controlled(
    installation: &werc_domain::connection::ClientInstallation, pid: u32, database: &str,
    control: &werc_application::ports::KeyScanControl<'_>,
) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
    windows::acquire_controlled(installation, pid, database, control)
}
pub fn database_salt(path: &str) -> AppResult<[u8; 16]> {
    windows::database_salt(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha512;
    #[test]
    fn controlled_scan_reports_only_byte_counts_and_stops_before_next_read() {
        use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};
        let cancel = AtomicBool::new(false);
        let updates = Mutex::new(Vec::new());
        let progress = |bytes| { updates.lock().unwrap().push(bytes); cancel.store(true, Ordering::SeqCst); };
        let control = werc_application::ports::KeyScanControl { cancel: &cancel, deadline: Instant::now()+Duration::from_secs(1), progress: &progress };
        let mut source = Synthetic(vec![0; CHUNK * 2]);
        let mut memory = ControlledMemory { memory: &mut source, control: &control, bytes: 0 };
        let result = scan(&mut memory, &[Region { base: 0x10000, size: CHUNK * 2 }], &page());
        assert_eq!(result.unwrap_err().code, "QUERY_CANCELED");
        assert_eq!(*updates.lock().unwrap(), vec![CHUNK as u64]);
        let expired = werc_application::ports::KeyScanControl { cancel: &AtomicBool::new(false), deadline: Instant::now(), progress: &|_| {} };
        assert_eq!(expired.check().unwrap_err().code, "KEY_SCAN_LIMIT");
    }
    #[test]
    fn config_cipher_support_is_independent_of_native_call_profiles() {
        for version in ["4.1.15.13", "4.1.15.56", "4.2.0.11", "4.2.0.12", "4.2.0.52"] {
            assert!(supports_config_cipher(version, "x86_64"));
            assert!(!supports_config_cipher(version, "x86"));
        }
        for version in ["4.0.0.1", "4.3.0.1", "4.2.", "4.2.x.52", "4.2.0.52.1"] {
            assert!(!supports_config_cipher(version, "x86_64"));
        }
    }
    pub(super) fn page() -> [u8; 4096] {
        let mut page = [0x35; 4096];
        let derived = werc_db_crypto::authentication_key(&[0x5a; 32], &[0x35; 16]).unwrap();
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(derived.as_ref()).unwrap();
        mac.update(&page[16..4032]);
        mac.update(&1u32.to_le_bytes());
        page[4032..].copy_from_slice(&mac.finalize().into_bytes());
        page
    }
    struct Synthetic(Vec<u8>);
    impl Memory for Synthetic {
        fn read(&mut self, address: u64, size: usize) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
            let Some(start) = address.checked_sub(0x10000) else {
                return Ok(None);
            };
            Ok(self
                .0
                .get(start as usize..start as usize + size)
                .map(|v| Zeroizing::new(v.to_vec())))
        }
    }
    #[test]
    fn bounded_blob_decode_requires_actual_first_page_authentication() {
        let raw = format!("x'{}{}'", "5a".repeat(32), "35".repeat(16));
        let blob: Vec<_> = raw
            .bytes()
            .enumerate()
            .map(|(i, b)| b ^ MASK[i % 32])
            .collect();
        assert_eq!(
            *key_from_blob(&blob, &page(), &mut 0).unwrap().unwrap(),
            [0x5a; 32]
        );
        let mut wrong = page();
        wrong[20] ^= 1;
        assert!(key_from_blob(&blob, &wrong, &mut 0).unwrap().is_none());
        assert!(
            key_from_blob(&vec![0; 1025], &page(), &mut 0)
                .unwrap()
                .is_none()
        );
        assert!(key_from_blob(&blob, &page(), &mut 2048).is_err());
    }
    #[test]
    fn follows_validated_object_chain_across_chunk_boundary() {
        let mut memory = Synthetic(vec![0; CHUNK + 512]);
        let name = CHUNK - 10;
        memory.0[name..name + NEEDLE.len()].copy_from_slice(NEEDLE);
        let node = 128;
        let config = 256;
        let data = 512;
        memory.0[node + 0x10..node + 0x18]
            .copy_from_slice(&(0x10000u64 + name as u64).to_le_bytes());
        memory.0[node + 0x18..node + 0x20].copy_from_slice(&(NEEDLE.len() as u64).to_le_bytes());
        memory.0[node + 0x28..node + 0x30]
            .copy_from_slice(&(0x10000u64 + config as u64).to_le_bytes());
        let raw = format!("x'{}'", "5a".repeat(32));
        memory.0[config + 0x90..config + 0x98]
            .copy_from_slice(&(0x10000u64 + data as u64).to_le_bytes());
        memory.0[config + 0x98..config + 0xa0].copy_from_slice(&(raw.len() as u64).to_le_bytes());
        for (i, b) in raw.bytes().enumerate() {
            memory.0[data + i] = b ^ MASK[i % 32];
        }
        let size = memory.0.len();
        assert_eq!(
            *scan(
                &mut memory,
                &[Region {
                    base: 0x10000,
                    size
                }],
                &page()
            )
            .unwrap(),
            [0x5a; 32]
        );
        memory.0[node + 0x18] ^= 1;
        assert!(
            scan(
                &mut memory,
                &[Region {
                    base: 0x10000,
                    size
                }],
                &page()
            )
            .is_err()
        );
        let mut limited = Scan {
            memory: &mut memory,
            remaining: 0,
            deadline: Instant::now() + Duration::from_secs(1),
            incomplete: false,
        };
        assert_eq!(limited.read(0x10000, 1).unwrap_err().code, "KEY_SCAN_LIMIT");
    }
}
