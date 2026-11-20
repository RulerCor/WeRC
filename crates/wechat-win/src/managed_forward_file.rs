//! Read the exact authenticated attachment from the current account's cache.
use super::*;
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    sync::atomic::{AtomicBool, Ordering},
};
use werc_domain::message_card::forward_file::ForwardFile;
use zeroize::Zeroizing;

fn missing() -> AppError {
    AppError::new(
        "FORWARD_FILE_CACHE_UNAVAILABLE",
        "微信本地没有与这条附件完整匹配的缓存，可先在微信打开附件后重新读取",
    )
}
fn check(cancel: &AtomicBool) -> AppResult<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(AppError::new("QUERY_CANCELED", "附件读取已取消"))
    } else {
        Ok(())
    }
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
fn pin_directories(path: &Path) -> AppResult<Vec<File>> {
    let ancestors: Vec<_> = path.ancestors().collect();
    let mut pins = Vec::new();
    for parent in ancestors.into_iter().rev() {
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
fn read_cache(
    root: &Path,
    descriptor: &ForwardFile,
    cancel: &AtomicBool,
) -> AppResult<Zeroizing<Vec<u8>>> {
    check(cancel)?;
    werc_domain::file::validate_file_name(&descriptor.file_name)?;
    if !(1..=werc_domain::file::MAX_FILE_BYTES as u64).contains(&descriptor.size) {
        return Err(missing());
    }
    let _root_pins = pin_directories(root)?;
    let mut bytes_examined = 0u64;
    for relative in ["msg/file", "FileStorage/File"] {
        let cache = root.join(relative);
        match std::fs::symlink_metadata(&cache) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(missing()),
            Ok(_) => {}
        }
        let _cache_pins = pin_directories(&cache)?;
        let mut folders = Vec::new();
        for (count, item) in std::fs::read_dir(&cache)
            .map_err(|_| missing())?
            .enumerate()
        {
            check(cancel)?;
            if count >= 512 {
                return Err(AppError::new(
                    "FORWARD_FILE_CACHE_LIMIT",
                    "附件缓存目录超过本次读取上限",
                ));
            }
            let item = item.map_err(|_| missing())?;
            if item.file_name().to_str().is_some_and(month) {
                folders.push(item.path());
            }
        }
        folders.sort_unstable();
        // Recent months first; the cache root is also a supported flat layout.
        folders.insert(0, cache.clone());
        for folder in folders.into_iter().rev() {
            check(cancel)?;
            let _folder_pins = pin_directories(&folder)?;
            let path = folder.join(descriptor.file_name.as_str());
            match std::fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(missing()),
                Ok(m) if !m.is_file() || m.file_attributes() & 0x400 != 0 => return Err(missing()),
                Ok(m) if m.len() != descriptor.size => continue,
                Ok(_) => {}
            }
            crate::data_sources::checked(&path, false)?;
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x00200000)
                .open(&path)
                .map_err(|_| missing())?;
            let before = file.metadata().map_err(|_| missing())?;
            if !before.is_file()
                || before.file_attributes() & 0x400 != 0
                || before.len() != descriptor.size
            {
                return Err(missing());
            }
            bytes_examined += descriptor.size;
            if bytes_examined > 64 * 1024 * 1024 {
                return Err(AppError::new(
                    "FORWARD_FILE_CACHE_LIMIT",
                    "同名附件过多，超过本次缓存校验上限",
                ));
            }
            let mut bytes = Zeroizing::new(Vec::new());
            (&mut file)
                .take(descriptor.size + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| missing())?;
            let after = file.metadata().map_err(|_| missing())?;
            if before.len() != after.len() || before.last_write_time() != after.last_write_time() {
                return Err(missing());
            }
            check(cancel)?;
            if descriptor.validate_bytes(&bytes).is_ok() {
                return Ok(bytes);
            }
        }
    }
    Err(missing())
}
impl ManagedWindowsWechatProvider {
    pub(super) fn read_forward_file(
        &self,
        account: &str,
        database: &str,
        source: &str,
        descriptor: &ForwardFile,
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
        let path = Path::new(database);
        crate::data_sources::checked(path, false)?;
        validate_live_source(path, &context.sender_id)?;
        let pin = crate::live_source::LiveSourcePin::open(path)?;
        pin.require(source)?;
        let root = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(missing)?;
        let bytes = read_cache(root, descriptor, cancel)?;
        provider.require_open()?;
        if provider.resolve_send_context(account, "filehelper")? != context {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "读取期间登录身份已变化",
            ));
        }
        crate::live_source::LiveSourcePin::open(path)?.require(source)?;
        check(cancel)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor() -> ForwardFile {
        werc_domain::message_card::forward_file::descriptor("<msg><appmsg><type>19</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"8\"><datatitle>自动化测试.txt</datatitle><datasize>3</datasize><fullmd5>900150983cd24fb0d6963f7d28e17f72</fullmd5></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>",&[0]).unwrap()
    }
    #[test]
    fn exact_cache_content_and_dynamic_unicode_paths() {
        let root =
            std::env::temp_dir().join(format!("werc-owned-附件 空格-{}", std::process::id()));
        assert!(!root.exists());
        let older = root.join("msg/file/2099-11");
        let newer = root.join("msg/file/2099-12");
        std::fs::create_dir_all(&older).unwrap();
        std::fs::create_dir_all(&newer).unwrap();
        let name = "自动化测试.txt";
        std::fs::write(older.join(name), b"abc").unwrap();
        std::fs::write(newer.join(name), b"abd").unwrap();
        let d = descriptor();
        let cancel = AtomicBool::new(false);
        assert_eq!(read_cache(&root, &d, &cancel).unwrap().as_slice(), b"abc");
        assert_eq!(std::fs::read(newer.join(name)).unwrap(), b"abd");
        assert_eq!(
            read_cache(&root, &d, &AtomicBool::new(true))
                .unwrap_err()
                .code,
            "QUERY_CANCELED"
        );
        std::fs::write(older.join(name), b"ab").unwrap();
        assert_eq!(
            read_cache(&root, &d, &cancel).unwrap_err().code,
            "FORWARD_FILE_CACHE_UNAVAILABLE"
        );
        let legacy = root.join("FileStorage/File/2001-01");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(name), b"abc").unwrap();
        assert_eq!(read_cache(&root, &d, &cancel).unwrap().as_slice(), b"abc");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_months_and_open_directory_rename_are_rejected() {
        for good in ["2000-01", "2026-11", "9999-12"] {
            assert!(month(good));
        }
        for bad in [
            "2026-00",
            "2026-13",
            "中文月份",
            "../2026",
            "2026-1",
            "1999-12",
        ] {
            assert!(!month(bad));
        }
        let root =
            std::env::temp_dir().join(format!("werc-owned-cache-pin-{}", std::process::id()));
        assert!(!root.exists());
        std::fs::create_dir(&root).unwrap();
        let pins = pin_directories(&root).unwrap();
        assert!(std::fs::rename(&root, root.with_extension("moved")).is_err());
        drop(pins);
        std::fs::remove_dir(&root).unwrap();
    }
}
