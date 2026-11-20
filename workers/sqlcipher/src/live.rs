//! Short read-only SQLCipher WAL view. No copy, immutable mode or checkpoint.
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::{File, OpenOptions},
    path::{Component, Path},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

type Result<T> = std::result::Result<T, &'static str>;

pub(crate) fn prepare_memory() -> Result<()> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn SetProcessWorkingSetSize(
                process: *mut std::ffi::c_void,
                minimum: usize,
                maximum: usize,
            ) -> i32;
        }
        // SQLCipher's Windows default permits roughly 1 MiB of VirtualLock.
        // Reserve a bounded working-set budget before its allocator starts;
        // the parent Job still enforces the 128 MiB process allocation ceiling.
        if unsafe {
            SetProcessWorkingSetSize(GetCurrentProcess(), 16 * 1024 * 1024, 64 * 1024 * 1024)
        } == 0
        {
            return Err("LIVE_MEMORY_BUDGET_UNAVAILABLE");
        }
    }
    Ok(())
}

fn pin(path: &Path, maximum: u64) -> Result<File> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("UNSAFE_SOURCE_PATH");
    }
    let text = path.to_str().ok_or("UNSAFE_SOURCE_PATH")?;
    if text.starts_with("\\\\") || text.starts_with("//") || text.chars().any(char::is_control) {
        return Err("UNSAFE_SOURCE_PATH");
    }
    #[cfg(windows)]
    if text.bytes().filter(|b| *b == b':').count() != 1 {
        return Err("UNSAFE_SOURCE_PATH");
    }
    for ancestor in path.ancestors() {
        let m = std::fs::symlink_metadata(ancestor).map_err(|_| "LIVE_SOURCE_UNAVAILABLE")?;
        if m.file_type().is_symlink() {
            return Err("UNSAFE_SOURCE_PATH");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err("UNSAFE_SOURCE_PATH");
            }
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Keep the selected objects alive while allowing the legitimate writer.
        // This is only a path pin; SQLite's own WAL locks supply consistency.
        options.share_mode(3).custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| "LIVE_SOURCE_UNAVAILABLE")?;
    let metadata = file.metadata().map_err(|_| "LIVE_SOURCE_UNAVAILABLE")?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err("LIVE_SOURCE_LIMIT");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("UNSAFE_SOURCE_PATH");
        }
    }
    Ok(file)
}

fn uri(path: &Path) -> Result<String> {
    let mut result = String::from("file:");
    for b in path
        .to_str()
        .ok_or("UNSAFE_SOURCE_PATH")?
        .replace('\\', "/")
        .bytes()
    {
        if b.is_ascii_alphanumeric() || b"/:._-".contains(&b) {
            result.push(b as char);
        } else {
            result.push_str(&format!("%{b:02X}"));
        }
    }
    result.push_str("?mode=ro&readonly_shm=1&cache=private");
    Ok(result)
}

pub(crate) struct ReadView {
    // Close SQLite before releasing the pinned DB/WAL/shm objects.
    pub(crate) connection: Connection,
    _pins: [File; 3],
    started: Instant,
}
impl ReadView {
    pub(crate) fn open(path: &Path, key: &str) -> Result<Self> {
        prepare_memory()?;
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("KEY_FORMAT_INVALID");
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("UNSAFE_SOURCE_PATH")?;
        // Live WAL is available only with both existing sidecars. Never create them.
        let pins = [
            pin(path, 1024 * 1024 * 1024)?,
            pin(
                &path.with_file_name(format!("{name}-wal")),
                256 * 1024 * 1024,
            )?,
            pin(
                &path.with_file_name(format!("{name}-shm")),
                16 * 1024 * 1024,
            )?,
        ];
        let c = Connection::open_with_flags(
            uri(path)?,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NOFOLLOW
                | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
        )
        .map_err(|_| "LIVE_OPEN_FAILED")?;
        let started = Instant::now();
        c.progress_handler(
            1000,
            Some(move || started.elapsed() > Duration::from_secs(2)),
        );
        c.busy_timeout(Duration::from_millis(250))
            .map_err(|_| "LIVE_OPEN_FAILED")?;
        let setup = Zeroizing::new(format!(
            "PRAGMA key=\"x'{key}'\"; PRAGMA cipher_compatibility=4; PRAGMA cipher_memory_security=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY; PRAGMA mmap_size=0; PRAGMA query_only=ON;"
        ));
        c.execute_batch(&setup).map_err(|_| "LIVE_CONFIG_FAILED")?;
        if !c.is_readonly("main").map_err(|_| "LIVE_CONFIG_FAILED")? {
            return Err("READ_ONLY_REQUIRED");
        }
        let mode: String = c
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .map_err(|_| "LIVE_KEY_OR_SCHEMA_FAILED")?;
        if mode != "wal" {
            return Err("LIVE_WAL_REQUIRED");
        }
        c.execute_batch("BEGIN")
            .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
        // Force a real read transaction now. No table content is returned here.
        c.query_row("SELECT count(*) FROM sqlite_schema", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|_| "LIVE_READ_FAILED")?;
        Ok(Self {
            connection: c,
            _pins: pins,
            started,
        })
    }
    pub(crate) fn check_deadline(&self) -> Result<()> {
        if self.started.elapsed() > Duration::from_secs(2) {
            Err("LIVE_READ_TIMEOUT")
        } else {
            Ok(())
        }
    }
}

pub(crate) fn probe(path: &Path, key: &str) -> Result<serde_json::Value> {
    let view = ReadView::open(path, key)?;
    let c = &view.connection;
    let count: i64 = c
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .map_err(|_| "LIVE_READ_FAILED")?;
    let pages: i64 = c
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .map_err(|_| "LIVE_READ_FAILED")?;
    let cipher: String = c
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .map_err(|_| "SQLCIPHER_REQUIRED")?;
    let sqlite: String = c
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .map_err(|_| "LIVE_READ_FAILED")?;
    c.execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    Ok(
        serde_json::json!({"schema_objects":count,"database_pages":pages,"cipher_version":cipher,"sqlite_version":sqlite,"read_only":true,"readonly_shm":true,"immutable":false,"rows_exported":0,"full_g1a_verified":false}),
    )
}
