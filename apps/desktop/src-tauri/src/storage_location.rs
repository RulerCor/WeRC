//! Product data relocation. The stable system directory owns the instance lock
//! and locator; the selected directory never changes where this lock is acquired.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use werc_domain::AppResult;

const LOCATOR: &str = "storage-location.json";
const MARKER: &str = ".werc-storage-owner";
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Locator {
    active: Option<PathBuf>,
    pending: Option<Pending>,
    previous: Option<PathBuf>,
    last_error: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    directory: PathBuf,
    id: String,
}
pub(super) struct Locations {
    base: PathBuf,
    pub data: PathBuf,
    locator: Mutex<Locator>,
}
#[derive(Serialize)]
pub(super) struct LocationStatus {
    directory: String,
    pending_directory: Option<String>,
    previous_directory: Option<String>,
    last_error: Option<String>,
}
fn io_error() -> AppError {
    AppError::new("STORAGE_IO", "无法读取或写入资料目录，请检查权限和可用空间")
}
fn ordinary(path: &Path) -> AppResult<fs::Metadata> {
    let meta = fs::symlink_metadata(path).map_err(|_| io_error())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(AppError::new("STORAGE_LINK", "资料路径不能经过目录链接"));
        }
    }
    if meta.file_type().is_symlink() {
        return Err(AppError::new("STORAGE_LINK", "资料路径不能经过链接"));
    }
    Ok(meta)
}
fn checked_directory(path: &Path) -> AppResult<PathBuf> {
    if !path.is_absolute() || path.components().any(|p| matches!(p, Component::ParentDir)) {
        return Err(AppError::new("INVALID_ARGUMENT", "请选择现有的本机目录"));
    }
    for parent in path.ancestors() {
        if !ordinary(parent)?.is_dir() {
            return Err(io_error());
        }
    }
    path.canonicalize().map_err(|_| io_error())
}
fn read_locator(base: &Path) -> AppResult<Locator> {
    match fs::read(base.join(LOCATOR)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            AppError::new(
                "STORAGE_LOCATION_INVALID",
                "资料位置配置损坏；未创建空资料库",
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Locator::default()),
        Err(_) => Err(io_error()),
    }
}
fn save_locator(base: &Path, locator: &Locator) -> AppResult<()> {
    let temp = base.join(format!("storage-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| io_error())?;
        file.write_all(&serde_json::to_vec_pretty(locator).map_err(|_| AppError::internal())?)
            .and_then(|_| file.sync_all())
            .map_err(|_| io_error())?;
        drop(file);
        preferences::replace(&temp, &base.join(LOCATOR))
    })();
    let _ = fs::remove_file(temp);
    result
}
fn roots(source: &Path) -> AppResult<Vec<PathBuf>> {
    // WebView state, the stable locator/lock, external exports and other files
    // are not part of the encrypted product store.
    let mut names = vec![
        PathBuf::from("state.sqlite3"),
        PathBuf::from("state.sqlite3-wal"),
        PathBuf::from("state.sqlite3-journal"),
        PathBuf::from("preferences.json"),
        PathBuf::from("desktop-lock.bin"),
        PathBuf::from("logs"),
        PathBuf::from("models"),
    ];
    if source.join("preferences.json").is_file() {
        let settings: serde_json::Value = serde_json::from_slice(
            &fs::read(source.join("preferences.json")).map_err(|_| io_error())?,
        )
        .map_err(|_| io_error())?;
        if let Some(s) = settings.get("model_directory").and_then(|v| v.as_str()) {
            let path = Path::new(s);
            if !path.is_absolute() {
                if path
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
                    || path.components().all(|c| matches!(c, Component::CurDir))
                {
                    return Err(AppError::new(
                        "STORAGE_MODEL_DIRECTORY",
                        "请先把模型目录设为应用数据目录内的独立子目录",
                    ));
                }
                names.push(path.to_owned());
            }
        }
    }
    for entry in fs::read_dir(source).map_err(|_| io_error())? {
        let entry = entry.map_err(|_| io_error())?;
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s.starts_with("state.sqlite3.pre-v") && s.ends_with(".sqlite3") {
            names.push(PathBuf::from(name));
        }
    }
    names.sort();
    names.dedup();
    let snapshot = names.clone();
    names.retain(|n| !snapshot.iter().any(|p| p != n && n.starts_with(p)));
    Ok(names)
}
fn inventory(source: &Path) -> AppResult<Vec<(PathBuf, u64)>> {
    let mut files = Vec::new();
    let mut pending = roots(source)?;
    while let Some(relative) = pending.pop() {
        let path = source.join(&relative);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(io_error()),
            Ok(_) => {}
        }
        let meta = ordinary(&path)?;
        if meta.is_file() {
            files.push((relative, meta.len()));
        } else if meta.is_dir() {
            for entry in fs::read_dir(path).map_err(|_| io_error())? {
                pending.push(relative.join(entry.map_err(|_| io_error())?.file_name()));
            }
        } else {
            return Err(io_error());
        }
        if pending.len() + files.len() > 100000 {
            return Err(AppError::new(
                "STORAGE_LIMIT",
                "资料目录项目过多，无法完成迁移",
            ));
        }
    }
    files.sort();
    Ok(files)
}
fn space(directory: &Path, required: u64) -> AppResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<_> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0;
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io_error());
        }
        if free < required.saturating_add(16 * 1024 * 1024) {
            return Err(AppError::new(
                "STORAGE_SPACE",
                "目标磁盘可用空间不足，未开始迁移",
            ));
        }
    }
    #[cfg(not(windows))]
    let _ = (directory, required);
    Ok(())
}
fn hash_file(path: &Path) -> AppResult<[u8; 32]> {
    let mut file = fs::File::open(path).map_err(|_| io_error())?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|_| io_error())?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hash.finalize().into())
}
fn migrate(source: &Path, target: &Pending, progress: &startup::Startup) -> AppResult<()> {
    let destination = checked_directory(&target.directory)?;
    if fs::read_to_string(destination.join(MARKER)).map_err(|_| io_error())? != target.id {
        return Err(AppError::new("STORAGE_OWNER", "目标目录标记不匹配，未迁移"));
    }
    // Each plan has a fresh private destination. Never merge with existing data.
    for entry in fs::read_dir(&destination).map_err(|_| io_error())? {
        if entry.map_err(|_| io_error())?.file_name() != MARKER {
            return Err(AppError::new(
                "STORAGE_NOT_EMPTY",
                "目标目录已含文件，原资料保留",
            ));
        }
    }
    let files = inventory(source)?;
    let total = files.iter().map(|(_, n)| n).sum();
    space(&destination, total)?;
    let mut copied = 0;
    progress.update("migrating", 0, total);
    progress.check()?;
    for (relative, expected) in &files {
        progress.check()?;
        let from = source.join(relative);
        let to = destination.join(relative);
        fs::create_dir_all(to.parent().ok_or_else(io_error)?).map_err(|_| io_error())?;
        let mut input = fs::File::open(&from).map_err(|_| io_error())?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&to)
            .map_err(|_| io_error())?;
        let mut hash = Sha256::new();
        let mut buf = vec![0; 1024 * 1024];
        let mut count = 0u64;
        loop {
            progress.check()?;
            let n = input.read(&mut buf).map_err(|_| io_error())?;
            if n == 0 {
                break;
            }
            output.write_all(&buf[..n]).map_err(|_| io_error())?;
            hash.update(&buf[..n]);
            count += n as u64;
            copied += n as u64;
            progress.update("migrating", copied, total);
        }
        output.sync_all().map_err(|_| io_error())?;
        drop(output);
        let digest: [u8; 32] = hash.finalize().into();
        if count != *expected || hash_file(&to)? != digest || hash_file(&from)? != digest {
            return Err(AppError::new(
                "STORAGE_CHANGED",
                "复制校验不一致，仍使用原资料目录",
            ));
        }
    }
    progress.check()?;
    progress.update("verifying", copied, total);
    if inventory(source)? != files {
        return Err(AppError::new(
            "STORAGE_CHANGED",
            "复制期间目录发生变化，仍使用原位置",
        ));
    }
    if destination.join("state.sqlite3").exists() {
        werc_infrastructure::SqliteRepository::verify_storage_copy(
            &destination.join("state.sqlite3"),
        )?;
    }
    Ok(())
}
impl Locations {
    // Called only after acquiring the stable instance.lock and before opening
    // preferences, logs, DB, native providers or background tasks.
    pub fn open(base: PathBuf, progress: &startup::Startup) -> AppResult<Self> {
        let base = checked_directory(&base)?;
        let mut locator = read_locator(&base)?;
        let mut data = match &locator.active {
            Some(path) => {
                let path = checked_directory(path)?;
                if !path.join(MARKER).is_file() {
                    return Err(AppError::new(
                        "STORAGE_UNAVAILABLE",
                        "已选择的资料目录不可用；未创建空资料库",
                    ));
                }
                path
            }
            None => base.clone(),
        };
        if let Some(pending) = locator.pending.take() {
            match migrate(&data, &pending, progress) {
                Ok(()) => {
                    locator.previous = Some(data.clone());
                    locator.active = Some(pending.directory.clone());
                    locator.last_error = None;
                    save_locator(&base, &locator)?;
                    data = pending.directory;
                }
                Err(error) => {
                    locator.last_error = Some(format!(
                        "{}：{}。原资料继续使用；未完成副本位于 {}",
                        error.code,
                        error.message,
                        pending.directory.display()
                    ));
                    save_locator(&base, &locator)?;
                }
            }
        }
        progress.update("starting", 0, 0);
        Ok(Self {
            base,
            data,
            locator: Mutex::new(locator),
        })
    }
    fn status(&self) -> AppResult<LocationStatus> {
        let value = self.locator.lock().map_err(|_| AppError::internal())?;
        Ok(LocationStatus {
            directory: self.data.to_string_lossy().into(),
            pending_directory: value
                .pending
                .as_ref()
                .map(|p| p.directory.to_string_lossy().into()),
            previous_directory: value.previous.as_ref().map(|p| p.to_string_lossy().into()),
            last_error: value.last_error.clone(),
        })
    }
    fn plan(&self, parent: &Path) -> AppResult<LocationStatus> {
        let parent = checked_directory(parent)?;
        if parent.starts_with(&self.data) {
            return Err(AppError::new(
                "STORAGE_NESTED",
                "请选择当前资料目录之外的位置",
            ));
        }
        let mut value = self.locator.lock().map_err(|_| AppError::internal())?;
        if value.pending.is_some() {
            return Err(AppError::new("STORAGE_PENDING", "请先取消已有迁移安排"));
        }
        space(&parent, inventory(&self.data)?.iter().map(|(_, n)| n).sum())?;
        let id = uuid::Uuid::new_v4().to_string();
        let directory = parent.join(format!("WeRC-data-{id}"));
        fs::create_dir(&directory).map_err(|_| io_error())?;
        let pending = Pending {
            directory: directory.clone(),
            id,
        };
        let result = (|| {
            let mut marker = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(MARKER))
                .map_err(|_| io_error())?;
            marker
                .write_all(pending.id.as_bytes())
                .and_then(|_| marker.sync_all())
                .map_err(|_| io_error())?;
            drop(marker);
            let mut next = value.clone();
            next.pending = Some(pending);
            next.last_error = None;
            save_locator(&self.base, &next)?;
            *value = next;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(directory.join(MARKER));
            let _ = fs::remove_dir(&directory);
        }
        result?;
        drop(value);
        self.status()
    }
    fn cancel(&self) -> AppResult<LocationStatus> {
        let mut value = self.locator.lock().map_err(|_| AppError::internal())?;
        if let Some(pending) = value.pending.as_ref() {
            let mut next = value.clone();
            next.pending = None;
            save_locator(&self.base, &next)?;
            // Remove only our empty reservation. Never recursively delete a path.
            if checked_directory(&pending.directory).is_ok()
                && fs::read_to_string(pending.directory.join(MARKER)).is_ok_and(|s| s == pending.id)
            {
                let _ = fs::remove_file(pending.directory.join(MARKER));
                let _ = fs::remove_dir(&pending.directory);
            }
            *value = next;
        }
        drop(value);
        self.status()
    }
}
#[tauri::command]
pub(super) fn storage_location(state: State<'_, Locations>) -> AppResult<LocationStatus> {
    state.status()
}
#[tauri::command]
pub(super) async fn plan_storage_migration(
    handle: tauri::AppHandle,
    parent: String,
) -> AppResult<LocationStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        handle.state::<Locations>().plan(Path::new(&parent))
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) fn cancel_storage_migration(state: State<'_, Locations>) -> AppResult<LocationStatus> {
    state.cancel()
}

#[cfg(test)]
mod tests {
    use super::*;
    use werc_application::ports::Repository;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "werc-migration-自动化测试-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn base(&self) -> PathBuf {
            let p = self.0.join("原资料");
            fs::create_dir(&p).unwrap();
            p
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn complete_store_migrates_and_reopens_without_touching_neighbors() {
        let f = Fixture::new();
        let base = f.base();
        let repo = SqliteRepository::open(&base.join("state.sqlite3")).unwrap();
        let account = repo.register_account("自动化测试").unwrap();
        drop(repo);
        fs::write(
            base.join("preferences.json"),
            br#"{"model_directory":"custom/models"}"#,
        )
        .unwrap();
        fs::create_dir_all(base.join("custom/models")).unwrap();
        fs::write(base.join("custom/models/ggml-owned.bin"), vec![7u8; 4096]).unwrap();
        fs::write(base.join("desktop-lock.bin"), b"owned-lock").unwrap();
        fs::write(base.join("keep.txt"), b"neighbor").unwrap();
        let progress = startup::Startup::default();
        let locations = Locations::open(base.clone(), &progress).unwrap();
        locations.plan(&f.0).unwrap();
        let dest = locations
            .locator
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .directory
            .clone();
        drop(locations);
        let migrated = Locations::open(base.clone(), &progress).unwrap();
        assert_eq!(migrated.data, dest);
        assert_eq!(
            fs::read(dest.join("custom/models/ggml-owned.bin")).unwrap(),
            vec![7; 4096]
        );
        assert_eq!(
            SqliteRepository::open(&dest.join("state.sqlite3"))
                .unwrap()
                .accounts()
                .unwrap()[0]
                .id,
            account.id
        );
        assert_eq!(fs::read(base.join("keep.txt")).unwrap(), b"neighbor");
        assert!(!dest.join("keep.txt").exists());
        assert!(base.join("state.sqlite3").exists());
        assert_eq!(Locations::open(base, &progress).unwrap().data, dest);
    }
    #[test]
    fn pending_cancel_and_copy_cancel_keep_original() {
        let f = Fixture::new();
        let base = f.base();
        fs::write(base.join("desktop-lock.bin"), b"original").unwrap();
        let progress = startup::Startup::default();
        let locations = Locations::open(base.clone(), &progress).unwrap();
        assert!(locations.plan(&base).is_err());
        locations.plan(&f.0).unwrap();
        let dest = locations
            .locator
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .directory
            .clone();
        locations.cancel().unwrap();
        assert!(!dest.exists());
        assert!(read_locator(&base).unwrap().pending.is_none());
        locations.plan(&f.0).unwrap();
        progress.update("migrating", 0, 100);
        // Same cancellation state used by the UI command; the next block aborts.
        progress.cancel_for_test();
        drop(locations);
        let canceled = Locations::open(base.clone(), &progress).unwrap();
        assert_eq!(canceled.data, base.canonicalize().unwrap());
        assert!(
            canceled
                .status()
                .unwrap()
                .last_error
                .unwrap()
                .contains("MIGRATION_CANCELED")
        );
        assert_eq!(
            fs::read(base.join("desktop-lock.bin")).unwrap(),
            b"original"
        );
    }
    #[test]
    fn occupied_destination_and_missing_active_root_never_replace_store() {
        let f = Fixture::new();
        let base = f.base();
        let progress = startup::Startup::default();
        let locations = Locations::open(base.clone(), &progress).unwrap();
        locations.plan(&f.0).unwrap();
        let dest = locations
            .locator
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .unwrap()
            .directory
            .clone();
        fs::write(dest.join("keep.txt"), b"neighbor").unwrap();
        drop(locations);
        let old = Locations::open(base.clone(), &progress).unwrap();
        assert_eq!(old.data, base.canonicalize().unwrap());
        assert_eq!(fs::read(dest.join("keep.txt")).unwrap(), b"neighbor");
        save_locator(
            &base,
            &Locator {
                active: Some(f.0.join("missing")),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(Locations::open(base, &progress).is_err());
        assert!(!f.0.join("missing").exists());
    }
}
