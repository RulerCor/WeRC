use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};
use werc_domain::{
    AppError, AppResult,
    connection::{DataSourceCandidate, DataSourceCatalog, DatabaseFile},
};

pub(crate) fn checked(path: &Path, directory: bool) -> AppResult<()> {
    let text = path
        .to_str()
        .ok_or_else(|| AppError::new("UNSAFE_PATH", "来源路径编码无效"))?;
    if !path.is_absolute()
        || text.len() > 32760
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.chars().any(char::is_control)
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(AppError::new("UNSAFE_PATH", "仅接受本机绝对来源路径"));
    }
    #[cfg(windows)]
    if text.chars().filter(|c| *c == ':').count() != 1 {
        return Err(AppError::new(
            "UNSAFE_PATH",
            "来源不能包含设备或备用数据流路径",
        ));
    }
    for ancestor in path.ancestors() {
        let m = std::fs::symlink_metadata(ancestor)
            .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "来源不存在或无法读取元数据"))?;
        if m.file_type().is_symlink() {
            return Err(AppError::new("UNSAFE_PATH", "来源不能经过符号链接"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err(AppError::new("UNSAFE_PATH", "来源不能经过重解析路径"));
            }
        }
    }
    if directory && !path.is_dir() {
        return Err(AppError::new("INVALID_ARGUMENT", "需要来源目录而不是文件"));
    }
    Ok(())
}
/// Local UI-selected read sources share the same reparse/UNC checks as databases.
pub fn validate_local_source(path:&Path)->AppResult<()>{checked(path,false)}
fn entries(path: &Path, limit: usize, budget: &mut usize) -> AppResult<Vec<PathBuf>> {
    checked(path, true)?;
    let mut result = Vec::new();
    for entry in std::fs::read_dir(path)
        .map_err(|_| AppError::new("SOURCE_ACCESS_DENIED", "无法列举来源目录"))?
        .take(limit + 1)
    {
        if result.len() == limit || *budget == 0 {
            return Err(AppError::new(
                "SOURCE_LIMIT_EXCEEDED",
                "来源目录项目超过有界检测上限",
            ));
        }
        *budget -= 1;
        result.push(
            entry
                .map_err(|_| AppError::new("SOURCE_ACCESS_DENIED", "无法读取目录项目"))?
                .path(),
        );
    }
    result.sort();
    Ok(result)
}
fn category(name: &str) -> &'static str {
    let name = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    if name.contains("session") {
        "sessions"
    } else if name.contains("contact") {
        "contacts"
    } else if name.contains("message") || name.starts_with("msg") {
        "messages"
    } else if name.contains("favorite") || name.contains("fav") {
        "favorites"
    } else if name.contains("sns") {
        "moments"
    } else {
        "other"
    }
}
fn candidate(path: &Path, budget: &mut usize) -> AppResult<Option<DataSourceCandidate>> {
    checked(path, true)?;
    let storage = path.join("db_storage");
    match std::fs::symlink_metadata(&storage) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(AppError::new(
                "SOURCE_ACCESS_DENIED",
                "无法读取数据库目录元数据",
            ));
        }
    }
    checked(&storage, true)?;
    let mut folders = vec![storage.clone()];
    for child in entries(&storage, 128, budget)? {
        checked(&child, false)?;
        if child.is_dir() {
            folders.push(child);
        }
    }
    let mut databases = Vec::new();
    for folder in folders {
        for file in entries(&folder, 1024, budget)? {
            if file
                .extension()
                .and_then(|v| v.to_str())
                .is_none_or(|v| !v.eq_ignore_ascii_case("db"))
            {
                continue;
            }
            checked(&file, false)?;
            let metadata = std::fs::metadata(&file)
                .map_err(|_| AppError::new("SOURCE_ACCESS_DENIED", "无法读取数据库元数据"))?;
            if !metadata.is_file() {
                continue;
            }
            let wal = file.with_file_name(format!(
                "{}-wal",
                file.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            ));
            let wal_present = match std::fs::symlink_metadata(&wal) {
                Ok(metadata) => {
                    checked(&wal, false)?;
                    metadata.is_file()
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => return Err(AppError::new("SOURCE_ACCESS_DENIED", "无法读取WAL元数据")),
            };
            let relative = file
                .strip_prefix(path)
                .map_err(|_| AppError::internal())?
                .to_str()
                .ok_or_else(AppError::internal)?
                .replace('\\', "/");
            databases.push(DatabaseFile {
                category: category(&relative).into(),
                relative_path: relative,
                size: metadata.len(),
                wal_present,
            });
            if databases.len() > 512 {
                return Err(AppError::new(
                    "SOURCE_LIMIT_EXCEEDED",
                    "单来源数据库数量超过512",
                ));
            }
        }
    }
    if databases.is_empty() {
        return Ok(None);
    }
    databases.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let canonical = path
        .canonicalize()
        .map_err(|_| AppError::new("SOURCE_ACCESS_DENIED", "来源无法规范化"))?;
    let canonical_text = canonical.to_str().ok_or_else(AppError::internal)?;
    let has_media_directory = ["FileStorage", "msg"]
        .iter()
        .any(|name| checked(&path.join(name), true).is_ok());
    Ok(Some(DataSourceCandidate {
        id: hex::encode(Sha256::digest(canonical_text.to_lowercase().as_bytes())),
        account_directory: path.to_str().ok_or_else(AppError::internal)?.into(),
        directory_name: path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
            .into(),
        layout: "wechat4_db_storage".into(),
        identity_verified: false,
        databases,
        has_media_directory,
    }))
}

/// 只列举显式根目录及下一层的数据库元数据，不打开数据库内容、密钥或进程内存。
pub fn inspect_root(path: &Path) -> AppResult<DataSourceCatalog> {
    checked(path, true)?;
    let mut catalog = DataSourceCatalog {
        roots_checked: 1,
        ..Default::default()
    };
    let mut budget = 16384;
    if let Some(source) = candidate(path, &mut budget)? {
        catalog.sources.push(source);
        return Ok(catalog);
    }
    for child in entries(path, 128, &mut budget)? {
        let m = std::fs::symlink_metadata(&child)
            .map_err(|_| AppError::new("SOURCE_ACCESS_DENIED", "来源目录元数据不可用"))?;
        if !m.is_dir() && !m.file_type().is_symlink() {
            continue;
        }
        match candidate(&child, &mut budget) {
            Ok(Some(source)) => catalog.sources.push(source),
            Ok(None) => {}
            Err(e) => {
                let exhausted = budget == 0;
                if catalog.warnings.len() < 32 {
                    catalog.warnings.push(e.code);
                }
                if exhausted {
                    break;
                }
            }
        }
    }
    Ok(catalog)
}

pub fn discover() -> AppResult<DataSourceCatalog> {
    let documents = documents_directory()?;
    let mut catalog = DataSourceCatalog::default();
    let mut sources = BTreeMap::new();
    for name in ["xwechat_files", "WeChat Files"] {
        let root = documents.join(name);
        catalog.roots_checked += 1;
        if std::fs::symlink_metadata(&root).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            continue;
        }
        match inspect_root(&root) {
            Ok(found) => {
                for s in found.sources {
                    sources.insert(s.id.clone(), s);
                }
                catalog.warnings.extend(found.warnings);
            }
            Err(e) => catalog.warnings.push(e.code),
        }
    }
    catalog.sources = sources.into_values().collect();
    catalog.warnings.truncate(32);
    Ok(catalog)
}
#[cfg(windows)]
fn documents_directory() -> AppResult<PathBuf> {
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_Documents, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath},
    };
    let mut pointer = std::ptr::null_mut();
    let status = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_Documents,
            KF_FLAG_DONT_VERIFY as u32,
            std::ptr::null_mut(),
            &mut pointer,
        )
    };
    if status < 0 || pointer.is_null() {
        return Err(AppError::new(
            "KNOWN_FOLDER_UNAVAILABLE",
            "系统文档目录不可用，请手动指定来源",
        ));
    }
    unsafe {
        let mut length = 0;
        while length < 32768 && *pointer.add(length) != 0 {
            length += 1;
        }
        let result = if length == 32768 {
            Err(AppError::internal())
        } else {
            use std::os::windows::ffi::OsStringExt;
            Ok(PathBuf::from(std::ffi::OsString::from_wide(
                std::slice::from_raw_parts(pointer, length),
            )))
        };
        CoTaskMemFree(pointer.cast());
        result
    }
}
#[cfg(not(windows))]
fn documents_directory() -> AppResult<PathBuf> {
    Err(AppError::new(
        "PLATFORM_UNSUPPORTED",
        "数据目录自动检测仅支持Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_relative_and_parent_paths() {
        assert!(inspect_root(Path::new("relative")).is_err());
        assert!(inspect_root(&std::env::temp_dir().join("..")).is_err());
        assert!(inspect_root(Path::new("//server/share")).is_err());
    }
    #[test]
    fn enforces_shared_directory_budget() {
        let mut budget = 0;
        assert_eq!(
            entries(&std::env::temp_dir(), 128, &mut budget)
                .unwrap_err()
                .code,
            "SOURCE_LIMIT_EXCEEDED"
        );
        assert_eq!(category("db_storage/message/Msg_0.db"), "messages");
        assert_eq!(category("db_storage/unknown.db"), "other");
    }
    #[test]
    fn detects_exact_and_suffixed_sources_without_guessing_identity() {
        let root = std::env::temp_dir().join(format!(
            "werc-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        for name in ["wxid_test", "custom_abcd"] {
            let folder = root.join(name).join("db_storage").join("session");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("session.db"), b"synthetic metadata only").unwrap();
        }
        let result = inspect_root(&root).unwrap();
        assert_eq!(result.sources.len(), 2);
        assert!(result.sources.iter().all(|s| !s.identity_verified));
        assert!(
            result
                .sources
                .iter()
                .all(|s| s.databases[0].category == "sessions")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
