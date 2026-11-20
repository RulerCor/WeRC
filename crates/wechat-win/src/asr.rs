//! Offline whisper.cpp process, resolved relative to the program's resources.
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use werc_domain::{AppError, AppResult, asr::*};
pub fn root() -> AppResult<PathBuf> {
    let exe = std::env::current_exe().map_err(|_| AppError::internal())?;
    let parent = exe.parent().ok_or_else(AppError::internal)?;
    if let Some(path) = std::env::var_os("WERC_ASR_DIRECTORY") {
        let path = PathBuf::from(path);
        return Ok(if path.is_absolute() {
            path
        } else {
            parent.join(path)
        });
    }
    let beside = parent.join("asr");
    if beside.is_dir() {
        return Ok(beside);
    }
    // Source debug/release executables share the target/asr resource directory.
    Ok(parent.parent().unwrap_or(parent).join("asr"))
}
static MODEL_DIRECTORY: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);
static MODEL_USE: std::sync::RwLock<()> = std::sync::RwLock::new(());
static EXECUTION: std::sync::RwLock<(&'static str, u32)> = std::sync::RwLock::new(("cpu", 2));
pub fn execution() -> AppResult<(&'static str, u32)> {
    EXECUTION
        .read()
        .map(|v| *v)
        .map_err(|_| AppError::internal())
}
pub fn whisper_runtime() -> AppResult<PathBuf> {
    let base = root()?;
    Ok(if execution()?.0 == "cuda" {
        base.join("cuda")
    } else {
        base
    })
}
pub fn cuda_backend_used(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes).lines().any(|line| {
        line.contains("whisper_backend_init_gpu:")
            && line.contains("using CUDA")
            && line.contains(" backend")
    })
}
pub fn configure_models_execution_with<T>(
    path: PathBuf,
    provider: &str,
    threads: u32,
    commit: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    let provider = match provider {
        "cpu" => "cpu",
        "cuda" => "cuda",
        _ => return Err(AppError::new("INVALID_ARGUMENT", "转写设备选项无效")),
    };
    if !(1..=32).contains(&threads) {
        return Err(AppError::new("INVALID_ARGUMENT", "转写线程数应为1至32"));
    }
    with_model_maintenance(|| {
        let mut directory = MODEL_DIRECTORY.write().map_err(|_| AppError::internal())?;
        let mut execution = EXECUTION.write().map_err(|_| AppError::internal())?;
        let result = commit()?;
        *directory = Some(path);
        *execution = (provider, threads);
        Ok(result)
    })
}
pub fn with_model_maintenance<T>(action: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
    let _guard = MODEL_USE
        .try_write()
        .map_err(|_| AppError::new("ASR_BUSY", "转写运行期间不能移动或移除模型"))?;
    action()
}
pub fn configure_models(path: PathBuf) -> AppResult<()> {
    configure_models_with(path, || Ok(()))
}
pub fn configure_models_with<T>(
    path: PathBuf,
    commit: impl FnOnce() -> AppResult<T>,
) -> AppResult<T> {
    with_model_maintenance(|| {
        let mut directory = MODEL_DIRECTORY.write().map_err(|_| AppError::internal())?;
        let result = commit()?;
        *directory = Some(path);
        Ok(result)
    })
}
pub fn model_directory() -> AppResult<PathBuf> {
    Ok(MODEL_DIRECTORY
        .read()
        .map_err(|_| AppError::internal())?
        .clone()
        .unwrap_or(root()?.join("models")))
}
fn model_path(model: &str) -> AppResult<PathBuf> {
    let user = model_directory()?.join(format!("ggml-{model}.bin"));
    if user.is_file() {
        return Ok(user);
    }
    Ok(root()?.join("models").join(format!("ggml-{model}.bin")))
}
fn valid_model(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 96
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
        && !s.contains("..")
}
pub fn status() -> AppResult<AsrStatus> {
    let root = root()?;
    let mut models = Vec::new();
    for directory in [root.join("models"), model_directory()?] {
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(id) = name
                    .strip_prefix("ggml-")
                    .and_then(|s| s.strip_suffix(".bin"))
                {
                    if valid_model(id)
                        && entry
                            .metadata()
                            .is_ok_and(|m| m.is_file() && m.len() > 1024 * 1024)
                    {
                        models.push(id.into());
                    }
                }
            }
        }
    }
    models.sort();
    models.dedup();
    let whisper_available =
        whisper_runtime()?.join("whisper-cli.exe").is_file() && !models.is_empty();
    if !whisper_available {
        models.clear();
    }
    let sherpa = super::sherpa_asr::catalog()?;
    models.extend(
        sherpa
            .models
            .into_iter()
            .filter(|m| m.installed && sherpa.runtime_available)
            .map(|m| m.id),
    );
    Ok(AsrStatus {
        available: whisper_available
            || (sherpa.runtime_available
                && models.iter().any(|id| super::sherpa_asr::is_model(id))),
        models,
        engine: "whisper.cpp/sherpa-onnx",
    })
}
pub fn transcribe(
    wav: &[u8],
    model: &str,
    language: &str,
    cancel: &AtomicBool,
) -> AppResult<Transcript> {
    let _model_use = MODEL_USE.read().map_err(|_| AppError::internal())?;
    if super::sherpa_asr::is_model(model) {
        return super::sherpa_asr::transcribe(wav, model, language, cancel);
    }
    let info = werc_domain::media::inspect_wav(wav)?;
    if !valid_model(model)
        || language.len() > 8
        || !language
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "模型或语言标识无效"));
    }
    let runtime = whisper_runtime()?;
    let executable = runtime.join("whisper-cli.exe");
    let (provider, threads) = execution()?;
    let model_path = model_path(model)?;
    if !executable.is_file() || !model_path.is_file() {
        return Err(AppError::new(
            "ASR_UNAVAILABLE",
            "离线转写组件或所选模型未安装",
        ));
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = std::env::temp_dir().join(format!("werc-asr-{}", hex::encode(random)));
    std::fs::create_dir(&directory)
        .map_err(|_| AppError::new("ASR_IO_FAILED", "无法创建转写临时目录"))?;
    let temp = Temp(directory);
    let input = temp.0.join("input.wav");
    let output = temp.0.join("output");
    let backend_log = temp.0.join("backend.log");
    std::fs::write(&input, wav).map_err(|_| AppError::new("ASR_IO_FAILED", "无法准备音频"))?;
    let mut command = Command::new(executable);
    command.current_dir(&runtime);
    command
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(&input)
        .arg("-l")
        .arg(language)
        .arg("-t")
        .arg(threads.to_string())
        .args(["-oj", "-of"])
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if provider == "cuda" {
            Stdio::from(std::fs::File::create(&backend_log).map_err(|_| AppError::internal())?)
        } else {
            Stdio::null()
        });
    if provider == "cpu" {
        command.args(["-ng", "-np"]);
    } else {
        super::cuda_devices::configure_command(&mut command)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| AppError::new("ASR_START_FAILED", "离线转写程序无法启动"))?;
    let start = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst)
            || start.elapsed() > Duration::from_secs(600)
            || std::fs::metadata(&backend_log).is_ok_and(|m| m.len() > 4 * 1024 * 1024)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new(
                if cancel.load(Ordering::SeqCst) {
                    "QUERY_CANCELED"
                } else {
                    "ASR_TIMEOUT"
                },
                "转写已取消或超时",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(AppError::new(
                        "ASR_FAILED",
                        "离线转写失败，请检查模型与音频格式",
                    ));
                }
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(80)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::new("ASR_FAILED", "无法读取转写进程状态"));
            }
        }
    }
    if provider == "cuda"
        && !cuda_backend_used(&std::fs::read(&backend_log).map_err(|_| AppError::internal())?)
    {
        return Err(AppError::new(
            "ASR_CUDA_NOT_ACTIVE",
            "CUDA未实际启用；请检查显卡驱动和CUDA组件，或在设置中改用CPU",
        ));
    }
    let result = output.with_extension("json");
    if std::fs::metadata(&result)
        .map_err(|_| AppError::new("ASR_FAILED", "转写没有产生结果"))?
        .len()
        > 4 * 1024 * 1024
    {
        return Err(AppError::new("ASR_INVALID_RESULT", "转写结果过大"));
    }
    let bytes = zeroize::Zeroizing::new(std::fs::read(result).map_err(|_| AppError::internal())?);
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("ASR_INVALID_RESULT", "转写结果无法解析"))?;
    let rows = value["transcription"]
        .as_array()
        .ok_or_else(|| AppError::new("ASR_INVALID_RESULT", "转写分段缺失"))?;
    let mut segments = Vec::new();
    let mut text = String::new();
    for row in rows {
        let from = row["offsets"]["from"]
            .as_u64()
            .ok_or_else(AppError::internal)?;
        let until = row["offsets"]["to"]
            .as_u64()
            .ok_or_else(AppError::internal)?;
        let part = row["text"]
            .as_str()
            .ok_or_else(AppError::internal)?
            .trim()
            .to_owned();
        if from > until || until > info.duration_ms + 30000 || part.len() > 65536 {
            return Err(AppError::new("ASR_INVALID_RESULT", "转写时间或文字无效"));
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&part);
        segments.push(Segment {
            from_ms: from,
            until_ms: until,
            text: part,
        });
    }
    Ok(Transcript {
        model: model.into(),
        language: value["result"]["language"]
            .as_str()
            .unwrap_or(language)
            .into(),
        text,
        segments,
    })
}

/// Hash installed bytes once per file version. This keeps large batch jobs fast
/// while replacing a model/engine under the same name invalidates its cache.
pub fn cache_identity(model: &str, cancel: &AtomicBool) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    use std::{
        collections::BTreeMap,
        io::Read,
        sync::{LazyLock, Mutex},
        time::SystemTime,
    };
    type Entry = (u64, Option<SystemTime>, String);
    static HASHES: LazyLock<Mutex<BTreeMap<PathBuf, Entry>>> =
        LazyLock::new(|| Mutex::new(BTreeMap::new()));
    if !valid_model(model) {
        return Err(AppError::new("INVALID_ARGUMENT", "模型标识无效"));
    }
    let root = whisper_runtime()?;
    let alternative = super::sherpa_asr::is_model(model);
    let mut paths = if alternative {
        super::sherpa_asr::identity_files(model)?
    } else {
        vec![root.join("whisper-cli.exe"), model_path(model)?]
    };
    if let Ok(entries) = std::fs::read_dir(&root) {
        if !alternative {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
                {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    let mut combined = Sha256::new();
    let (provider, threads) = execution()?;
    combined.update(provider.as_bytes());
    combined.update(threads.to_le_bytes());
    for path in paths {
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::new("QUERY_CANCELED", "转写已取消"));
        }
        let meta = std::fs::metadata(&path)
            .map_err(|_| AppError::new("ASR_UNAVAILABLE", "转写模型或组件未安装"))?;
        let version = (meta.len(), meta.modified().ok());
        let mut cached = HASHES.lock().map_err(|_| AppError::internal())?;
        let hash = if let Some((_, _, hash)) = cached
            .get(&path)
            .filter(|v| v.0 == version.0 && v.1 == version.1 && version.1.is_some())
        {
            hash.clone()
        } else {
            let mut file = std::fs::File::open(&path)
                .map_err(|_| AppError::new("ASR_IO_FAILED", "无法读取转写组件"))?;
            let mut digest = Sha256::new();
            let mut buffer = vec![0u8; 1024 * 1024];
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err(AppError::new("QUERY_CANCELED", "转写已取消"));
                }
                let n = file
                    .read(&mut buffer)
                    .map_err(|_| AppError::new("ASR_IO_FAILED", "无法读取转写组件"))?;
                if n == 0 {
                    break;
                }
                digest.update(&buffer[..n]);
            }
            let after = file.metadata().map_err(|_| AppError::internal())?;
            if version != (after.len(), after.modified().ok()) {
                return Err(AppError::new("ASR_MODEL_CHANGED", "转写组件读取期间改变"));
            }
            let hash = hex::encode(digest.finalize());
            if cached.len() > 64 {
                cached.clear();
            }
            cached.insert(path.clone(), (version.0, version.1, hash.clone()));
            hash
        };
        let model_root = model_directory()?;
        let relative = path
            .strip_prefix(&model_root)
            .or_else(|_| path.strip_prefix(&root))
            .unwrap_or(&path);
        combined.update(relative.to_string_lossy().as_bytes());
        combined.update([0]);
        combined.update(hash.as_bytes());
    }
    Ok(hex::encode(combined.finalize()))
}
