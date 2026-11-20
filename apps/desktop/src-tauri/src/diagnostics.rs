use super::*;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
const FILES: [&str; 3] = [
    "diagnostics.jsonl",
    "diagnostics.1.jsonl",
    "diagnostics.2.jsonl",
];
pub(super) struct Diagnostics {
    directory: PathBuf,
    last: Mutex<Option<(String, String, u64)>>,
    session_id: String,
    sequence: std::sync::atomic::AtomicU64,
    started: std::time::Instant,
}
fn valid_stage(stage: &str) -> bool {
    matches!(
        stage,
        "receive"
            | "backfill"
            | "send"
            | "task"
            | "message-preservation"
            | "moment-preservation"
            | "lifecycle"
            | "desktop"
            | "asr"
    )
}
#[derive(Serialize)]
pub(super) struct Storage {
    directory: String,
    pub(super) bytes: u64,
    pub(super) files: usize,
    enabled: bool,
}
impl Diagnostics {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory: directory.join("logs"),
            last: Mutex::new(None),
            session_id: uuid::Uuid::new_v4().to_string(),
            sequence: std::sync::atomic::AtomicU64::new(0),
            started: std::time::Instant::now(),
        }
    }
    fn checked(&self) -> Result<(), AppError> {
        // Only three fixed, application-owned log files are ever rotated/removed.
        if !self.directory.exists() {
            std::fs::create_dir(&self.directory)
                .map_err(|_| AppError::new("LOG_IO", "无法创建本地日志目录"))?;
        }
        let meta = std::fs::symlink_metadata(&self.directory).map_err(|_| AppError::internal())?;
        if !meta.is_dir() || linked(&meta) {
            return Err(AppError::new(
                "LOG_DIRECTORY_INVALID",
                "日志目录不是普通本机目录",
            ));
        }
        for name in FILES {
            match std::fs::symlink_metadata(self.directory.join(name)) {
                Ok(meta) if !meta.is_file() || linked(&meta) => {
                    return Err(AppError::new(
                        "LOG_FILE_INVALID",
                        "日志文件被替换为非普通文件",
                    ));
                }
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(AppError::new("LOG_IO", "无法检查本地日志"));
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn record(&self, stage: &str, code: &str) -> Result<(), AppError> {
        self.event(stage, code, "error", None)
    }
    fn event(
        &self,
        stage: &str,
        code: &str,
        level: &str,
        elapsed_ms: Option<u64>,
    ) -> Result<(), AppError> {
        if !valid_stage(stage)
            || !matches!(level, "info" | "error")
            || code.is_empty()
            || code.len() > 128
            || !code
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        {
            return Err(AppError::new("LOG_CODE_INVALID", "诊断只接受结构化错误码"));
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AppError::internal())?
            .as_secs();
        let mut guard = self.last.lock().map_err(|_| AppError::internal())?;
        if level == "error"
            && guard
                .as_ref()
                .is_some_and(|(s, c, t)| s == stage && c == code && seconds.saturating_sub(*t) < 60)
        {
            return Ok(());
        }
        self.checked()?;
        if std::fs::metadata(self.directory.join(FILES[0])).is_ok_and(|m| m.len() >= 1024 * 1024) {
            if self.directory.join(FILES[2]).exists() {
                std::fs::remove_file(self.directory.join(FILES[2]))
                    .map_err(|_| AppError::new("LOG_IO", "无法轮换旧日志"))?;
            }
            for (a, b) in [(FILES[1], FILES[2]), (FILES[0], FILES[1])] {
                if self.directory.join(a).exists() {
                    std::fs::rename(self.directory.join(a), self.directory.join(b))
                        .map_err(|_| AppError::new("LOG_IO", "无法轮换日志"))?;
                }
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join(FILES[0]))
            .map_err(|_| AppError::new("LOG_IO", "无法写入本地日志"))?;
        let mut line = serde_json::to_vec(
            &serde_json::json!({"unix_seconds":seconds,"stage":stage,"code":code,"level":level,"session_id":self.session_id,"event_id":self.sequence.fetch_add(1,std::sync::atomic::Ordering::Relaxed)+1,"elapsed_ms":elapsed_ms,"uptime_ms":self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,"version":env!("CARGO_PKG_VERSION")}),
        )
        .map_err(|_| AppError::internal())?;
        line.push(b'\n');
        file.write_all(&line)
            .map_err(|_| AppError::new("LOG_IO", "日志写入失败"))?;
        *guard = Some((stage.into(), code.into(), seconds));
        Ok(())
    }
    pub(super) fn clear(&self) -> Result<(), AppError> {
        let mut guard = self.last.lock().map_err(|_| AppError::internal())?;
        self.checked()?;
        for name in FILES {
            match std::fs::remove_file(self.directory.join(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(AppError::new("LOG_IO", "部分日志未清理，请重试")),
            }
        }
        *guard = None;
        Ok(())
    }
    pub(super) fn storage(&self, enabled: bool) -> Result<Storage, AppError> {
        let _guard = self.last.lock().map_err(|_| AppError::internal())?;
        self.checked()?;
        let mut bytes = 0;
        let mut files = 0;
        for name in FILES {
            if let Ok(m) = std::fs::metadata(self.directory.join(name)) {
                bytes += m.len();
                files += 1;
            }
        }
        Ok(Storage {
            directory: self.directory.to_string_lossy().into_owned(),
            bytes,
            files,
            enabled,
        })
    }
}
fn linked(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
pub(super) fn record(handle: &tauri::AppHandle, stage: &str, error: &AppError) {
    if handle
        .state::<preferences::PreferencesStore>()
        .diagnostics_logging()
    {
        let _ = handle.state::<Diagnostics>().record(stage, &error.code);
    }
}
pub(super) fn event(
    handle: &tauri::AppHandle,
    stage: &str,
    code: &str,
    elapsed: Option<std::time::Duration>,
) {
    if handle
        .try_state::<preferences::PreferencesStore>()
        .is_some_and(|s| s.diagnostics_logging())
    {
        if let Some(log) = handle.try_state::<Diagnostics>() {
            let _ = log.event(
                stage,
                code,
                "info",
                elapsed.map(|d| d.as_millis().min(u64::MAX as u128) as u64),
            );
        }
    }
}
pub(super) fn install_panic_hook(handle: tauri::AppHandle) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic payloads and source paths can contain user content; record only a fixed code.
        if handle
            .try_state::<preferences::PreferencesStore>()
            .is_some_and(|s| s.diagnostics_logging())
        {
            if let Some(log) = handle.try_state::<Diagnostics>() {
                let _ = log.record("lifecycle", "UNHANDLED_PANIC");
            }
        }
        previous(info);
    }));
}
#[tauri::command]
pub(super) fn diagnostic_frontend_error(
    handle: tauri::AppHandle,
    code: String,
) -> Result<(), AppError> {
    if !matches!(
        code.as_str(),
        "FRONTEND_ERROR" | "FRONTEND_UNHANDLED_REJECTION"
    ) {
        return Err(AppError::new("INVALID_ARGUMENT", "未知界面故障类型"));
    }
    record(&handle, "desktop", &AppError::new(&code, "界面异常"));
    Ok(())
}
#[tauri::command]
pub(super) fn diagnostic_log_storage(handle: tauri::AppHandle) -> Result<Storage, AppError> {
    handle.state::<Diagnostics>().storage(
        handle
            .state::<preferences::PreferencesStore>()
            .diagnostics_logging(),
    )
}
#[tauri::command]
pub(super) fn clear_diagnostic_logs(handle: tauri::AppHandle) -> Result<Storage, AppError> {
    let state = handle.state::<Diagnostics>();
    state.clear()?;
    state.storage(
        handle
            .state::<preferences::PreferencesStore>()
            .diagnostics_logging(),
    )
}
#[derive(Serialize)]
pub(super) struct Component {
    name: &'static str,
    status: &'static str,
}
#[derive(Serialize)]
pub(super) struct Report {
    product: &'static str,
    version: &'static str,
    os: &'static str,
    architecture: &'static str,
    native_connected: bool,
    native_error: Option<String>,
    api_enabled: bool,
    asr_provider: String,
    gpu: Option<gpu::Detection>,
    components: Vec<Component>,
    log_records: Vec<serde_json::Value>,
}
fn component(name: &'static str, expected: Option<&str>, directory: &std::path::Path) -> Component {
    let status = (|| {
        let Some(expected) = expected.filter(|s| s.len() == 64) else {
            return "not_assembled";
        };
        let path = directory.join(name);
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            return "missing";
        };
        if !metadata.is_file() || linked(&metadata) || metadata.len() > 256 * 1024 * 1024 {
            return "invalid_file";
        }
        let Ok(mut file) = std::fs::File::open(path) else {
            return "unreadable";
        };
        let mut hash = Sha256::new();
        let mut buf = [0; 65536];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hash.update(&buf[..n]),
                Err(_) => return "unreadable",
            }
        }
        if hex::encode(hash.finalize()) == expected {
            "verified"
        } else {
            "hash_mismatch"
        }
    })();
    Component { name, status }
}
fn report(handle: &tauri::AppHandle) -> Result<Report, AppError> {
    let executable = std::env::current_exe().map_err(|_| AppError::internal())?;
    let directory = executable.parent().ok_or_else(AppError::internal)?;
    let state = handle.state::<DesktopState>();
    let native = state.native_provider.status()?;
    let diagnostics = handle.state::<Diagnostics>();
    let _guard = diagnostics.last.lock().map_err(|_| AppError::internal())?;
    diagnostics.checked()?;
    let mut records = Vec::new();
    // Reports project only logger-owned fields, even if a user edited the file.
    for name in FILES.into_iter().rev() {
        if let Ok(file) = std::fs::File::open(diagnostics.directory.join(name)) {
            let mut text = String::new();
            file.take(1024 * 1024 + 4096)
                .read_to_string(&mut text)
                .map_err(|_| AppError::new("LOG_IO", "日志编码无法读取"))?;
            for line in text.lines() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    let code = v["code"].as_str().unwrap_or("");
                    let stage = v["stage"].as_str().unwrap_or("");
                    if !code.is_empty()
                        && code.len() <= 128
                        && code
                            .bytes()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
                        && valid_stage(stage)
                    {
                        let mut entry = serde_json::json!({"unix_seconds":v["unix_seconds"].as_u64(),"stage":stage,"code":code});
                        for field in ["event_id", "elapsed_ms", "uptime_ms"] {
                            if let Some(n) = v[field].as_u64() {
                                entry[field] = n.into();
                            }
                        }
                        if let Some(id) = v["session_id"]
                            .as_str()
                            .filter(|s| uuid::Uuid::parse_str(s).is_ok())
                        {
                            entry["session_id"] = id.into();
                        }
                        if let Some(level) = v["level"]
                            .as_str()
                            .filter(|s| matches!(*s, "info" | "error"))
                        {
                            entry["level"] = level.into();
                        }
                        records.push(entry);
                    }
                }
            }
        }
    }
    if records.len() > 200 {
        records.drain(..records.len() - 200);
    }
    Ok(Report {
        product: "WeRC_Mon3trBot",
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        native_connected: native.connected,
        native_error: native.error_code,
        api_enabled: state.application.status().api_enabled,
        asr_provider: werc_wechat_win::asr::execution()?.0.into(),
        gpu: gpu::detect().ok(),
        components: vec![
            component(
                "werc-message-host.exe",
                option_env!("WERC_MESSAGE_HOST_SHA256"),
                directory,
            ),
            component(
                "werc-sqlcipher-worker.exe",
                option_env!("WERC_SQLCIPHER_WORKER_SHA256"),
                directory,
            ),
            component(
                "werc-voice-codec.exe",
                option_env!("WERC_VOICE_CODEC_SHA256"),
                directory,
            ),
            component(
                "werc-emoticon-codec.exe",
                option_env!("WERC_EMOTICON_CODEC_SHA256"),
                directory,
            ),
        ],
        log_records: records,
    })
}
#[tauri::command]
pub(super) async fn diagnostic_report(handle: tauri::AppHandle) -> Result<Report, AppError> {
    tauri::async_runtime::spawn_blocking(move || report(&handle))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn export_diagnostic_report(
    handle: tauri::AppHandle,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    tauri::async_runtime::spawn_blocking(move || {
        let output =
            serde_json::to_vec_pretty(&report(&handle)?).map_err(|_| AppError::internal())?;
        export_file::write_new(&export_file::destination(&path, &downloads)?, &output)?;
        Ok(output.len())
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_rotate_deduplicate_and_preserve_user_data() {
        let root = std::env::temp_dir().join(format!("werc-log-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("state.sqlite3"), b"owned-user-data").unwrap();
        let log = Diagnostics::new(root.clone());
        log.record("receive", "TEST_FAILURE").unwrap();
        log.record("receive", "TEST_FAILURE").unwrap();
        assert_eq!(
            std::fs::read_to_string(log.directory.join(FILES[0]))
                .unwrap()
                .lines()
                .count(),
            1
        );
        log.record("message-preservation", "PRESERVATION_FAILED")
            .unwrap();
        log.record("moment-preservation", "PRESERVATION_FAILED")
            .unwrap();
        log.event("lifecycle", "STARTED", "info", Some(250))
            .unwrap();
        let text = std::fs::read_to_string(log.directory.join(FILES[0])).unwrap();
        let last: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(last["elapsed_ms"], 250);
        assert_eq!(last["event_id"], 4);
        assert!(uuid::Uuid::parse_str(last["session_id"].as_str().unwrap()).is_ok());
        std::fs::write(log.directory.join(FILES[0]), vec![b' '; 1024 * 1024]).unwrap();
        log.record("task", "TEST_FAILURE").unwrap();
        assert_eq!(log.storage(true).unwrap().files, 2);
        assert!(log.record("private text", "TEST_FAILURE").is_err());
        assert!(log.record("receive", "private text").is_err());
        std::fs::write(log.directory.join("keep.txt"), b"owned-neighbor").unwrap();
        log.clear().unwrap();
        assert_eq!(log.storage(false).unwrap().files, 0);
        assert_eq!(
            std::fs::read(root.join("state.sqlite3")).unwrap(),
            b"owned-user-data"
        );
        assert_eq!(
            std::fs::read(log.directory.join("keep.txt")).unwrap(),
            b"owned-neighbor"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
