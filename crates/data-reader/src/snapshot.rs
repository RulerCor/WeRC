use crate::codec::ReadResult;
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Component, Path},
};
use zeroize::Zeroizing;

pub(crate) fn safe_path(path: &Path) -> ReadResult<()> {
    let text = path.to_str().ok_or("UNSAFE_PATH")?;
    if !path.is_absolute()
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.chars().any(char::is_control)
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("UNSAFE_PATH");
    }
    #[cfg(windows)]
    if text.chars().filter(|c| *c == ':').count() != 1 {
        return Err("UNSAFE_PATH");
    }
    for ancestor in path.ancestors() {
        let m = std::fs::symlink_metadata(ancestor).map_err(|_| "SOURCE_NOT_FOUND")?;
        if m.file_type().is_symlink() {
            return Err("UNSAFE_PATH");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err("UNSAFE_PATH");
            }
        }
    }
    Ok(())
}
fn open(path: &Path) -> ReadResult<File> {
    safe_path(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // 排除写入和删除共享；活跃写者存在时拒绝而非复制不一致文件。
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| "SOURCE_BUSY_OR_DENIED")?;
    if !file.metadata().map_err(|_| "SOURCE_READ_FAILED")?.is_file() {
        return Err("UNSAFE_PATH");
    }
    Ok(file)
}
fn contents(file: &mut File, maximum: u64) -> ReadResult<Zeroizing<Vec<u8>>> {
    let size = file.metadata().map_err(|_| "SOURCE_READ_FAILED")?.len();
    if size > maximum {
        return Err("DATABASE_LIMIT_EXCEEDED");
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(size as usize));
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "SOURCE_READ_FAILED")?;
    if bytes.len() as u64 != size {
        return Err("SOURCE_CHANGED");
    }
    Ok(bytes)
}
// Source handles deny write/delete sharing on Windows and stay alive through SQLite traversal.
pub const MAX_DB_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_WAL_BYTES: u64 = 128 * 1024 * 1024;
pub const TOTAL_BUDGET_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) struct Source {
    pub database: File,
    pub wal: Option<File>,
    pub database_len: u64,
    pub wal_len: u64,
    wal_path: std::path::PathBuf,
}
impl Source {
    pub fn open(path: &Path) -> ReadResult<Self> {
        let database = open(path)?;
        let wal_path = path.with_file_name(format!(
            "{}-wal",
            path.file_name()
                .and_then(|v| v.to_str())
                .ok_or("UNSAFE_PATH")?
        ));
        let wal = match std::fs::symlink_metadata(&wal_path) {
            Ok(_) => Some(open(&wal_path)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("SOURCE_BUSY_OR_DENIED"),
        };
        let database_len = database.metadata().map_err(|_| "SOURCE_READ_FAILED")?.len();
        let wal_len = wal
            .as_ref()
            .map(|f| f.metadata().map(|m| m.len()))
            .transpose()
            .map_err(|_| "SOURCE_READ_FAILED")?
            .unwrap_or(0);
        if database_len > MAX_DB_BYTES || wal_len > MAX_WAL_BYTES {
            return Err("DATABASE_LIMIT_EXCEEDED");
        }
        #[cfg(not(windows))]
        if wal_len != 0 {
            return Err("PLATFORM_SNAPSHOT_UNSUPPORTED");
        }
        Ok(Self {
            database,
            wal,
            database_len,
            wal_len,
            wal_path,
        })
    }
    pub fn digest(&mut self) -> ReadResult<String> {
        use sha2::{Digest, Sha256};
        use std::io::{Seek, SeekFrom};
        if self.wal.is_none() {
            match std::fs::symlink_metadata(&self.wal_path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                _ => return Err("SOURCE_CHANGED"),
            }
        }
        let mut hash = Sha256::new();
        hash.update(self.database_len.to_le_bytes());
        let mut buffer = Zeroizing::new([0u8; 65536]);
        for (file, expected) in std::iter::once((&mut self.database, self.database_len))
            .chain(self.wal.as_mut().map(|f| (f, self.wal_len)))
        {
            if file.metadata().map_err(|_| "SOURCE_READ_FAILED")?.len() != expected {
                return Err("SOURCE_CHANGED");
            }
            file.seek(SeekFrom::Start(0))
                .map_err(|_| "SOURCE_READ_FAILED")?;
            let mut count = 0u64;
            loop {
                let n = file
                    .read(buffer.as_mut())
                    .map_err(|_| "SOURCE_READ_FAILED")?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                if count > expected {
                    return Err("SOURCE_CHANGED");
                }
                hash.update(&buffer[..n]);
            }
            if count != expected {
                return Err("SOURCE_CHANGED");
            }
        }
        Ok(hex::encode(hash.finalize()))
    }
}

type SnapshotBytes = (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>);
pub fn snapshot(path: &Path) -> ReadResult<SnapshotBytes> {
    let mut database = open(path)?;
    let wal_path = path.with_file_name(format!(
        "{}-wal",
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or("UNSAFE_PATH")?
    ));
    let mut wal = match std::fs::symlink_metadata(&wal_path) {
        Ok(_) => Some(open(&wal_path)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("SOURCE_BUSY_OR_DENIED"),
    };
    let bytes = contents(&mut database, 64 * 1024 * 1024)?;
    let wal_bytes = if let Some(w) = &mut wal {
        contents(w, 32 * 1024 * 1024)?
    } else {
        Zeroizing::new(Vec::new())
    };
    // Windows句柄在这两个读取完成之前保持打开，期间禁止其他写者/删除者。
    #[cfg(not(windows))]
    if wal_bytes.len() > 0 {
        return Err("PLATFORM_SNAPSHOT_UNSUPPORTED");
    }
    Ok((bytes, wal_bytes))
}
