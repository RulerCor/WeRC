use super::*;
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    sync::atomic::{AtomicBool, Ordering},
};
use werc_domain::received_emoticon::ReceivedEmoticonDescriptor;
use zeroize::Zeroizing;

fn check(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(AppError::new("QUERY_CANCELED", "表情读取已取消"))
    } else {
        Ok(())
    }
}
fn missing() -> AppError {
    AppError::new(
        "EMOTICON_CACHE_UNAVAILABLE",
        "微信本地尚无可读取的此表情缓存",
    )
}
fn month(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..].iter().all(u8::is_ascii_digit)
        && name[..4]
            .parse::<u16>()
            .is_ok_and(|y| (2000..=9999).contains(&y))
        && name[5..].parse::<u8>().is_ok_and(|m| (1..=12).contains(&m))
}

// Keep every ancestor open without delete sharing through the cache read. The
// relative layout is derived from a verified message DB, never from UI paths.
fn pin_directories(path: &Path) -> AppResult<Vec<File>> {
    let mut pins = Vec::new();
    for parent in path.ancestors() {
        crate::data_sources::checked(parent, true)?;
        let file = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(parent)
            .map_err(|_| missing())?;
        let m = file.metadata().map_err(|_| missing())?;
        if !m.is_dir() || m.file_attributes() & 0x400 != 0 {
            return Err(missing());
        }
        pins.push(file);
    }
    Ok(pins)
}

fn read_cache(root: &Path, md5: &str, cancel: &AtomicBool) -> AppResult<Zeroizing<Vec<u8>>> {
    if md5.len() != 32 || !md5.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(missing());
    }
    let cache = root.join("cache");
    let _pins = pin_directories(&cache)?;
    let mut months = Vec::new();
    for (count, item) in std::fs::read_dir(&cache)
        .map_err(|_| missing())?
        .enumerate()
    {
        check(cancel)?;
        if count >= 256 {
            return Err(AppError::new(
                "EMOTICON_CACHE_LIMIT",
                "表情缓存目录超过读取上限",
            ));
        }
        let item = item.map_err(|_| missing())?;
        if item.file_name().to_str().is_some_and(month) {
            months.push(item.path());
        }
    }
    months.sort_unstable();
    for folder in months.into_iter().rev() {
        check(cancel)?;
        let path = folder.join("Emoticon").join(&md5[..2]).join(md5);
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(missing()),
            Ok(_) => {}
        }
        let _parents = pin_directories(path.parent().ok_or_else(missing)?)?;
        crate::data_sources::checked(&path, false)?;
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x00200000)
            .open(&path)
            .map_err(|_| missing())?;
        let m = file.metadata().map_err(|_| missing())?;
        let limit = werc_domain::emoticon::MAX_EMOTICON_BYTES + 16;
        if !m.is_file()
            || m.file_attributes() & 0x400 != 0
            || m.len() < 16
            || m.len() > limit as u64
            || !m.len().is_multiple_of(16)
        {
            return Err(missing());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        (&mut file)
            .take((limit + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| missing())?;
        let after = file.metadata().map_err(|_| missing())?;
        if bytes.len() != m.len() as usize
            || after.len() != m.len()
            || after.last_write_time() != m.last_write_time()
        {
            return Err(AppError::new(
                "EMOTICON_CACHE_CHANGED",
                "读取期间表情缓存变化，请重新读取",
            ));
        }
        check(cancel)?;
        return Ok(bytes);
    }
    Err(missing())
}

impl ManagedWindowsWechatProvider {
    pub(super) fn read_emoticon(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        descriptor: &ReceivedEmoticonDescriptor,
        cancel: &AtomicBool,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.read_emoticon_source(
            account,
            Some((database, source_fingerprint)),
            None,
            descriptor,
            cancel,
        )
    }
    pub(super) fn read_emoticon_source(
        &self,
        account: &str,
        source: Option<(&str, &str)>,
        expected: Option<&SendContext>,
        descriptor: &ReceivedEmoticonDescriptor,
        cancel: &AtomicBool,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        check(cancel)?;
        let session = self.session.read().map_err(|_| AppError::internal())?;
        let provider = session
            .provider
            .as_ref()
            .ok_or_else(|| AppError::new("NATIVE_NOT_CONNECTED", "当前连接已断开"))?;
        provider.require_open()?;
        let context = provider.resolve_send_context(account, "filehelper")?;
        let pid = session.pid.ok_or_else(AppError::internal)?;
        if expected.is_some_and(|v| *v != context) {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "当前登录身份已变化",
            ));
        }
        let (root, _message_pin) = if let Some((database, source_fingerprint)) = source {
            let path = Path::new(database);
            crate::data_sources::checked(path, false)?;
            validate_live_source(path, &context.sender_id)?;
            let pin = crate::live_source::LiveSourcePin::open(path)?;
            pin.require(source_fingerprint)?;
            (
                path.parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .ok_or_else(missing)?
                    .to_path_buf(),
                Some(pin),
            )
        } else {
            let catalog = crate::data_sources::discover()?;
            let roots: Vec<_> = catalog
                .sources
                .iter()
                .filter(|s| {
                    let n = Path::new(&s.account_directory)
                        .file_name()
                        .and_then(|n| n.to_str());
                    n.is_some_and(|n| {
                        n == context.sender_id
                            || n.strip_prefix(&context.sender_id)
                                .is_some_and(|suffix| suffix.starts_with('_') && suffix.len() > 1)
                    })
                })
                .collect();
            if roots.len() != 1 {
                return Err(AppError::new(
                    "LIVE_SOURCE_IDENTITY_MISMATCH",
                    "当前账号的缓存来源不唯一或不可用",
                ));
            }
            (std::path::PathBuf::from(&roots[0].account_directory), None)
        };
        let cache = read_cache(&root, &descriptor.md5, cancel)?;
        let installations = crate::discovery::discover()?.installations;
        let clients: Vec<_> = installations
            .iter()
            .filter(|i| i.process_ids.contains(&pid) && i.sha256 == context.installation_sha256)
            .collect();
        if clients.len() != 1 {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "当前微信进程身份已变化",
            ));
        }
        let bytes = crate::key_acquisition::read_emoticon(
            clients[0],
            pid,
            &context.sender_id,
            &cache,
            &descriptor.md5,
            descriptor.converted_cache,
            cancel,
        )?;
        descriptor.validate_bytes(&bytes)?;
        check(cancel)?;
        provider.require_open()?;
        if provider.resolve_send_context(account, "filehelper")? != context {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "读取期间登录身份已变化",
            ));
        }
        if let Some((database, source_fingerprint)) = source {
            crate::live_source::LiveSourcePin::open(Path::new(database))?
                .require(source_fingerprint)?;
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_months_are_dynamic_and_exact() {
        for good in ["2024-01", "2026-11", "2099-12"] {
            assert!(month(good));
        }
        for bad in [
            "2026-00",
            "2026-13",
            "2026-1",
            "../2026",
            "2026-11-extra",
            "中文月份",
            "1999-12",
        ] {
            assert!(!month(bad));
        }
    }
    #[test]
    fn relative_cache_layout_works_in_unicode_directory_without_fixed_month_or_drive() {
        let root =
            std::env::temp_dir().join(format!("werc-owned-表情 空格-{}", std::process::id()));
        assert!(!root.exists());
        let folder = root.join("cache/2099-12/Emoticon/df");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("df4813390984d6155aabb73ea22dade0");
        let cipher = crate::emoticon_cache::tests::encoded(1234567890, "owned-account");
        std::fs::write(&file, &cipher).unwrap();
        assert_eq!(
            read_cache(
                &root,
                "df4813390984d6155aabb73ea22dade0",
                &AtomicBool::new(false)
            )
            .unwrap()
            .as_slice(),
            cipher
        );
        assert!(read_cache(&root, "../../invalid", &AtomicBool::new(false)).is_err());
        assert_eq!(
            read_cache(
                &root,
                "df4813390984d6155aabb73ea22dade0",
                &AtomicBool::new(true)
            )
            .unwrap_err()
            .code,
            "QUERY_CANCELED"
        );
        assert_eq!(std::fs::read(&file).unwrap(), cipher);
        std::fs::write(&file, [0u8; 17]).unwrap();
        assert!(
            read_cache(
                &root,
                "df4813390984d6155aabb73ea22dade0",
                &AtomicBool::new(false)
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
