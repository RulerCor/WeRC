use std::path::{Component, Path, PathBuf};
use werc_domain::{AppError, AppResult};

/// Relative export names are rooted in the OS Downloads folder, never in cwd.
pub fn destination(path: &str, downloads: &Path) -> AppResult<PathBuf> {
    if path.trim().is_empty() {
        return Err(AppError::new("INVALID_ARGUMENT", "请填写文件名"));
    }
    let supplied = Path::new(path);
    let destination = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        if supplied
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "相对文件名不能离开下载目录",
            ));
        }
        downloads.join(supplied)
    };
    let path = destination.to_str().ok_or_else(AppError::internal)?;
    if !destination.is_absolute()
        || path.starts_with("\\\\")
        || path.starts_with("//")
        || path.chars().any(char::is_control)
        || destination
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::new("INVALID_ARGUMENT", "需要本机完整导出路径"));
    }
    for component in destination.components() {
        if let std::path::Component::Normal(name) = component {
            let name = name
                .to_str()
                .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "导出路径编码无效"))?;
            let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
            if name.contains([':', '<', '>', '"', '|', '?', '*'])
                || name.ends_with([' ', '.'])
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
            {
                return Err(AppError::new(
                    "INVALID_ARGUMENT",
                    "导出路径包含保留名称或非法字符",
                ));
            }
        }
    }
    for parent in destination.ancestors().skip(1) {
        let meta = std::fs::symlink_metadata(parent)
            .map_err(|_| AppError::new("INVALID_ARGUMENT", "目标目录不存在"))?;
        if meta.file_type().is_symlink() {
            return Err(AppError::new("INVALID_ARGUMENT", "目标不能经过链接目录"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(AppError::new("INVALID_ARGUMENT", "目标不能经过重解析目录"));
            }
        }
    }
    Ok(destination)
}

pub fn write_new(destination: &Path, bytes: &[u8]) -> AppResult<usize> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| {
            AppError::new(
                "EXPORT_DESTINATION_EXISTS_OR_DENIED",
                "目标已存在或无法创建；不会覆盖已有文件",
            )
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            AppError::new(
                "EXPORT_WRITE_FAILED",
                "导出未完成，可能留下不完整的新文件；来源未修改",
            )
        })?;
    Ok(bytes.len())
}

/// Stream to a sibling temporary file; only publish a fully flushed export.
pub fn stream_new<T>(
    destination: &Path,
    write: impl FnOnce(&mut std::fs::File) -> AppResult<T>,
) -> AppResult<T> {
    stream_with_policy(destination, ConflictPolicy::Refuse, write).map(|(result, _, _)| result)
}

#[derive(Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    #[default]
    Refuse,
    KeepBoth,
    Replace,
    SkipIdentical,
}

/// Generate once, then publish according to the user's explicit conflict choice.
/// Existing exports survive generation, cancellation and sync failures.
pub fn stream_with_policy<T>(
    destination: &Path,
    policy: ConflictPolicy,
    write: impl FnOnce(&mut std::fs::File) -> AppResult<T>,
) -> AppResult<(T, PathBuf, bool)> {
    struct Temporary(PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let temporary = destination.with_extension(format!(
        "{}.werc-partial",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| AppError::internal())?
            .as_nanos()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "无法创建导出临时文件"))?;
    let cleanup = Temporary(temporary);
    let result = write(&mut file).and_then(|value| {
        file.sync_all()
            .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "无法完成导出文件同步"))?;
        Ok(value)
    });
    // Close the handle before cleanup runs on an error (required on Windows).
    drop(file);
    let result = result?;
    if matches!(policy, ConflictPolicy::SkipIdentical) && destination.exists() {
        if same_bytes(&cleanup.0, destination)? {
            return Ok((result, destination.to_owned(), true));
        }
        return Err(AppError::new(
            "EXPORT_CONTENT_CHANGED",
            "已有文件内容不同，请选择保留副本或替换文件",
        ));
    }
    let mut selected = destination.to_owned();
    if matches!(policy, ConflictPolicy::KeepBoth) {
        // publish_new refuses collisions atomically, including another exporter
        // creating the same candidate after our existence check.
        let stem = destination
            .file_stem()
            .ok_or_else(AppError::internal)?
            .to_string_lossy();
        let extension = destination
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();
        for copy in 0..1000 {
            if copy > 0 {
                selected = destination.with_file_name(format!("{stem} ({copy}){extension}"));
            }
            if selected.exists() {
                continue;
            }
            match publish_new(&cleanup.0, &selected) {
                Ok(()) => return Ok((result, selected, false)),
                Err(_) if selected.exists() => continue,
                Err(e) => return Err(e),
            }
        }
        return Err(AppError::new(
            "EXPORT_COPY_LIMIT",
            "同名副本过多，请选择新的文件名",
        ));
    }
    if matches!(policy, ConflictPolicy::Replace) {
        if let Ok(meta) = std::fs::symlink_metadata(destination) {
            if !meta.is_file() || !super::storage::ordinary(&meta) {
                return Err(AppError::new(
                    "EXPORT_DESTINATION_INVALID",
                    "只能替换普通文件",
                ));
            }
        }
        publish_replace(&cleanup.0, destination)?;
    } else {
        publish_new(&cleanup.0, destination)?;
    }
    Ok((result, selected, false))
}

fn same_bytes(left: &Path, right: &Path) -> AppResult<bool> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(right)
        .map_err(|_| AppError::new("EXPORT_READ_FAILED", "无法核对已有文件"))?;
    if !meta.is_file() || !super::storage::ordinary(&meta) {
        return Err(AppError::new(
            "EXPORT_DESTINATION_INVALID",
            "只能核对普通文件",
        ));
    }
    let compare = || -> std::io::Result<bool> {
        let mut a = std::fs::File::open(left)?;
        let mut b = std::fs::File::open(right)?;
        if a.metadata()?.len() != b.metadata()?.len() {
            return Ok(false);
        }
        let mut x = [0u8; 65536];
        let mut y = [0u8; 65536];
        loop {
            let n = a.read(&mut x)?;
            if n == 0 {
                return Ok(b.read(&mut y[..1])? == 0);
            }
            match b.read_exact(&mut y[..n]) {
                Ok(()) if x[..n] == y[..n] => (),
                Ok(()) => return Ok(false),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(e) => return Err(e),
            }
        }
    };
    compare().map_err(|_| AppError::new("EXPORT_READ_FAILED", "无法核对已有文件"))
}

fn publish_replace(temporary: &Path, destination: &Path) -> AppResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(AppError::new(
                "EXPORT_REPLACE_FAILED",
                "无法替换已有文件，原文件保持不变",
            ));
        }
    }
    #[cfg(not(windows))]
    std::fs::rename(temporary, destination)
        .map_err(|_| AppError::new("EXPORT_REPLACE_FAILED", "无法替换已有文件"))?;
    Ok(())
}

pub fn publish_new(temporary: &Path, destination: &Path) -> AppResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let from: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            return Err(AppError::new(
                "EXPORT_DESTINATION_EXISTS_OR_DENIED",
                "目标已存在或无法发布导出文件",
            ));
        }
    }
    #[cfg(not(windows))]
    std::fs::hard_link(temporary, destination).map_err(|_| {
        AppError::new(
            "EXPORT_DESTINATION_EXISTS_OR_DENIED",
            "目标已存在或无法发布导出文件",
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflict_choices_preserve_old_file_until_complete_and_return_actual_copy() {
        use std::io::Write;
        let root =
            std::env::temp_dir().join(format!("werc-owned-conflict-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("自动化测试.txt");
        std::fs::write(&path, b"old").unwrap();
        let write = |file: &mut std::fs::File| -> AppResult<usize> {
            file.write_all(b"new").unwrap();
            Ok(3)
        };
        let (_, copy, skipped) =
            stream_with_policy(&path, ConflictPolicy::KeepBoth, write).unwrap();
        assert_eq!(copy, root.join("自动化测试 (1).txt"));
        assert!(!skipped);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read(&copy).unwrap(), b"new");
        assert_eq!(
            stream_with_policy(&path, ConflictPolicy::SkipIdentical, write)
                .unwrap_err()
                .code,
            "EXPORT_CONTENT_CHANGED"
        );
        assert!(
            stream_with_policy(&copy, ConflictPolicy::SkipIdentical, write)
                .unwrap()
                .2
        );
        let failure: AppResult<((), PathBuf, bool)> =
            stream_with_policy(&path, ConflictPolicy::Replace, |file| {
                file.write_all(b"partial").unwrap();
                Err(AppError::new("OWNED_FAILURE", "自动化测试"))
            });
        assert!(failure.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        stream_with_policy(&path, ConflictPolicy::Replace, write).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(copy).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn failed_stream_closes_handle_before_cleanup_and_never_publishes_partial() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("werc-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let destination = root.join("自动化测试.txt");
        let result: AppResult<()> = stream_new(&destination, |file| {
            file.write_all(b"partial").unwrap();
            Err(AppError::new("OWNED_FAILURE", "自动化测试"))
        });
        assert!(result.is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        stream_new(&destination, |file| {
            file.write_all(b"complete").unwrap();
            Ok(())
        })
        .unwrap();
        assert!(
            stream_new(&destination, |file| {
                file.write_all(b"replacement").unwrap();
                Ok(())
            })
            .is_err()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_file(destination).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn relative_destination_is_independent_of_working_directory_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!(
            "werc-export-中文 空格-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let target = destination("自动化测试.txt", &root).unwrap();
        assert_eq!(target, root.join("自动化测试.txt"));
        assert!(destination("../escape.txt", &root).is_err());
        assert!(destination("NUL.txt", &root).is_err());
        assert!(destination(" ", &root).is_err());
        #[cfg(windows)]
        for name in ["C:relative.txt", "\\rooted.txt", "folder:stream.txt"] {
            assert!(destination(name, &root).is_err());
        }
        assert_eq!(write_new(&target, b"owned").unwrap(), 5);
        assert!(write_new(&target, b"replacement").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"owned");
        std::fs::remove_file(target).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
