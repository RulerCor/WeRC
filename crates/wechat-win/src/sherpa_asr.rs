//! Local sherpa-onnx CLI adapter. Model paths are relative to one installed bundle.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use werc_domain::{
    AppError, AppResult,
    asr::{Segment, Transcript},
};
pub const MODELS: &[(&str, &str)] = &[
    ("sherpa-qwen3-0.6b", "Qwen3-ASR 0.6B"),
    ("sherpa-qwen3-1.7b", "Qwen3-ASR 1.7B"),
    ("sherpa-funasr-nano", "Fun-ASR Nano"),
    ("sherpa-paraformer", "Paraformer"),
    ("sherpa-zipformer", "Zipformer"),
];
pub fn is_model(id: &str) -> bool {
    MODELS.iter().any(|(name, _)| *name == id)
}
pub fn runtime_directory_for(provider: &str) -> AppResult<PathBuf> {
    let name = match provider {
        "cpu" => "sherpa",
        "cuda" => "sherpa-cuda",
        _ => return Err(invalid("未知计算设备")),
    };
    let installed = super::asr::model_directory()?.join(".runtimes").join(name);
    Ok(if installed.is_dir() {
        installed
    } else {
        super::asr::root()?.join(name)
    })
}
pub fn runtime_directory() -> AppResult<PathBuf> {
    runtime_directory_for(super::asr::execution()?.0)
}
pub fn is_runtime(id: &str) -> bool {
    matches!(id, "runtime" | "runtime-cuda")
}
pub fn import_runtime_directory(id: &str) -> AppResult<PathBuf> {
    Ok(super::asr::model_directory()?
        .join(".runtimes")
        .join(if id == "runtime-cuda" {
            "sherpa-cuda"
        } else {
            "sherpa"
        }))
}
pub fn model_directory(id: &str) -> AppResult<PathBuf> {
    if !is_model(id) {
        return Err(invalid("模型标识无效"));
    }
    Ok(super::asr::model_directory()?.join(id))
}
fn available_model_directory(id: &str) -> AppResult<PathBuf> {
    let user = model_directory(id)?;
    Ok(if user.exists() { user } else { super::asr::root()?.join("models").join(id) })
}
fn invalid(message: &str) -> AppError {
    AppError::new("ASR_MODEL_INVALID", message)
}
fn plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !reparse(&m))
}
pub fn reparse(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
/// Inventory also includes ONNX external data and tokenizer files in cache identity.
pub fn bundle_files(directory: &Path) -> AppResult<Vec<PathBuf>> {
    fn walk(root: &Path, at: &Path, depth: usize, output: &mut Vec<PathBuf>) -> AppResult<()> {
        if depth > 6 {
            return Err(invalid("模型子目录过深"));
        }
        let meta = std::fs::symlink_metadata(at).map_err(|_| invalid("无法读取模型目录"))?;
        if !meta.is_dir() || reparse(&meta) {
            return Err(invalid("模型目录不能使用链接或重解析点"));
        }
        for item in std::fs::read_dir(at).map_err(|_| invalid("无法枚举模型文件"))? {
            let item = item.map_err(|_| invalid("无法枚举模型文件"))?;
            let path = item.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|_| invalid("模型文件不可读"))?;
            if reparse(&meta) {
                return Err(invalid("模型包含链接或重解析点"));
            }
            if meta.is_dir() {
                walk(root, &path, depth + 1, output)?;
            } else if meta.is_file() {
                if output.len() >= 1024 || meta.len() > 16 * 1024 * 1024 * 1024 {
                    return Err(invalid("模型文件数量或大小超过限制"));
                }
                output.push(
                    path.strip_prefix(root)
                        .map_err(|_| invalid("模型路径无效"))?
                        .to_path_buf(),
                );
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(directory, directory, 0, &mut files)?;
    files.sort();
    Ok(files)
}
fn select(files: &[PathBuf], patterns: &[&str]) -> AppResult<PathBuf> {
    let mut patterns = patterns.to_vec();
    if super::asr::execution()?.0 == "cuda" {
        patterns.sort_by_key(|p| p.contains("int8") || p.contains("quant"));
    }
    for pattern in patterns {
        let choices = files
            .iter()
            .filter(|p| {
                p.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                    if let Some((start, end)) = pattern.split_once('*') {
                        s.starts_with(start) && s.ends_with(end)
                    } else {
                        s == pattern
                    }
                })
            })
            .collect::<Vec<_>>();
        if choices.len() == 1 {
            return Ok(choices[0].clone());
        }
        if choices.len() > 1 {
            return Err(invalid("模型包包含多个同类权重，请保留一套匹配的权重"));
        }
    }
    Err(invalid("模型包缺少必要权重或词表文件"))
}
pub fn configuration(id: &str, directory: &Path) -> AppResult<BTreeMap<String, PathBuf>> {
    if !is_model(id) {
        return Err(invalid("模型标识无效"));
    }
    let files = bundle_files(directory)?;
    let mut result = BTreeMap::new();
    let pairs: &[(&str, &[&str])] = match id {
        "sherpa-qwen3-0.6b" | "sherpa-qwen3-1.7b" => &[
            (
                "qwen3-asr-conv-frontend",
                &["conv_frontend.int8.onnx", "conv_frontend.onnx"],
            ),
            ("qwen3-asr-encoder", &["encoder.int8.onnx", "encoder.onnx"]),
            ("qwen3-asr-decoder", &["decoder.int8.onnx", "decoder.onnx"]),
        ],
        "sherpa-funasr-nano" => &[
            (
                "funasr-nano-encoder-adaptor",
                &["encoder_adaptor.int8.onnx", "encoder_adaptor.onnx"],
            ),
            ("funasr-nano-llm", &["llm.int8.onnx", "llm.onnx"]),
            (
                "funasr-nano-embedding",
                &["embedding.int8.onnx", "embedding.onnx"],
            ),
        ],
        "sherpa-paraformer" => &[
            (
                "paraformer",
                &["model.int8.onnx", "model.onnx", "model_quant.onnx"],
            ),
            ("tokens", &["tokens.txt"]),
        ],
        "sherpa-zipformer" => &[
            ("encoder", &["encoder*.int8.onnx", "encoder*.onnx"]),
            ("decoder", &["decoder*.int8.onnx", "decoder*.onnx"]),
            ("joiner", &["joiner*.int8.onnx", "joiner*.onnx"]),
            ("tokens", &["tokens.txt"]),
        ],
        _ => return Err(invalid("模型标识无效")),
    };
    for (key, patterns) in pairs {
        let path = select(&files, patterns)?;
        if std::fs::metadata(directory.join(&path))
            .map_err(|_| invalid("模型文件不可读"))?
            .len()
            == 0
        {
            return Err(invalid("模型文件为空"));
        }
        result.insert((*key).into(), path);
    }
    if id.starts_with("sherpa-qwen3") || id == "sherpa-funasr-nano" {
        let vocab = select(&files, &["vocab.json"])?;
        let tokenizer = vocab.parent().unwrap_or(Path::new(""));
        for file in [
            "vocab.json",
            "merges.txt",
            if id == "sherpa-funasr-nano" {
                "tokenizer.json"
            } else {
                "tokenizer_config.json"
            },
        ] {
            if !plain_file(&directory.join(tokenizer).join(file)) {
                return Err(invalid("分词器文件不完整"));
            }
        }
        result.insert(
            if id == "sherpa-funasr-nano" {
                "funasr-nano-tokenizer"
            } else {
                "qwen3-asr-tokenizer"
            }
            .into(),
            tokenizer.to_owned(),
        );
    }
    Ok(result)
}
#[derive(Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub label: String,
    pub installed: bool,
    pub removable: bool,
    pub reason: Option<String>,
}
#[derive(Serialize)]
pub struct Catalog {
    pub runtime_available: bool,
    pub cpu_runtime_available: bool,
    pub cuda_runtime_available: bool,
    pub provider: String,
    pub removable_runtime: bool,
    pub removable_cuda_runtime: bool,
    pub models: Vec<ModelStatus>,
}
pub fn catalog() -> AppResult<Catalog> {
    let runtime_available = plain_file(&runtime_directory()?.join("sherpa-onnx-offline.exe"));
    let models = MODELS
        .iter()
        .map(|(id, label)| {
            let check = available_model_directory(id).and_then(|dir| configuration(id, &dir));
            ModelStatus {
                id: (*id).into(),
                label: (*label).into(),
                installed: check.is_ok(),
                removable: model_directory(id).is_ok_and(|dir| dir.join("werc-import.json").is_file()),
                reason: check.err().map(|e| e.message),
            }
        })
        .collect();
    Ok(Catalog {
        runtime_available,
        cpu_runtime_available: plain_file(
            &runtime_directory_for("cpu")?.join("sherpa-onnx-offline.exe"),
        ),
        cuda_runtime_available: plain_file(
            &runtime_directory_for("cuda")?.join("sherpa-onnx-offline.exe"),
        ),
        provider: super::asr::execution()?.0.into(),
        removable_runtime: import_runtime_directory("runtime")?.join("werc-import.json").is_file(),
        removable_cuda_runtime: import_runtime_directory("runtime-cuda")?.join("werc-import.json").is_file(),
        models,
    })
}
pub fn identity_files(id: &str) -> AppResult<Vec<PathBuf>> {
    let directory = available_model_directory(id)?;
    configuration(id, &directory)?;
    let mut files = bundle_files(&directory)?
        .into_iter()
        .map(|p| directory.join(p))
        .collect::<Vec<_>>();
    let runtime = runtime_directory()?;
    for item in std::fs::read_dir(&runtime)
        .map_err(|_| AppError::new("ASR_UNAVAILABLE", "sherpa-onnx运行时未安装"))?
    {
        let p = item.map_err(|_| AppError::internal())?.path();
        if p.file_name()
            .is_some_and(|n| n == "sherpa-onnx-offline.exe")
            || p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        {
            if !plain_file(&p) {
                return Err(invalid("运行时文件无效"));
            }
            files.push(p);
        }
    }
    if !plain_file(&runtime.join("sherpa-onnx-offline.exe")) {
        return Err(AppError::new("ASR_UNAVAILABLE", "sherpa-onnx运行时未安装"));
    }
    Ok(files)
}
pub fn transcribe(
    wav: &[u8],
    id: &str,
    language: &str,
    cancel: &AtomicBool,
) -> AppResult<Transcript> {
    let info = werc_domain::media::inspect_wav(wav)?;
    let directory = available_model_directory(id)?;
    let config = configuration(id, &directory)?;
    if !matches!(language, "auto" | "zh" | "en" | "ja" | "ko" | "yue") {
        return Err(AppError::new("INVALID_ARGUMENT", "此引擎的语言选项无效"));
    }
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
    let temp = Temp(std::env::temp_dir().join(format!("werc-sherpa-{}", hex::encode(random))));
    std::fs::create_dir(&temp.0).map_err(|_| AppError::new("ASR_IO_FAILED", "无法创建临时目录"))?;
    let input = temp.0.join("input.wav");
    std::fs::write(&input, wav).map_err(|_| AppError::new("ASR_IO_FAILED", "无法准备音频"))?;
    let output = temp.0.join("result.json");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(|_| AppError::internal())?;
    let runtime = runtime_directory()?;
    let mut command = Command::new(runtime.join("sherpa-onnx-offline.exe"));
    let (provider, threads) = super::asr::execution()?;
    let provider_option = if provider == "cuda" {
        super::cuda_devices::configure_command(&mut command)?;
        let configuration = temp.0.join("cuda.config");
        std::fs::write(
            &configuration,
            format!(
                "ProfilingFilePrefix={}\n",
                temp.0.join("execution").display()
            ),
        )
        .map_err(|_| AppError::internal())?;
        format!("cuda:{}", configuration.display())
    } else {
        provider.into()
    };
    command
        .current_dir(&runtime)
        .arg(format!("--provider={provider_option}"))
        .arg(format!("--num-threads={threads}"))
        .args(["--decoding-method=greedy_search", "--debug=false"]);
    for (key, path) in config {
        command.arg(format!(
            "--{key}={}",
            directory.join(path).to_string_lossy()
        ));
    }
    if id == "sherpa-funasr-nano" && language != "auto" {
        command.arg(format!(
            "--funasr-nano-language={}",
            match language {
                "zh" => "Chinese",
                "en" => "English",
                "ja" => "Japanese",
                "ko" => "Korean",
                "yue" => "Cantonese",
                _ => unreachable!(),
            }
        ));
    }
    command
        .arg(&input)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::new("QUERY_CANCELED", "转写已取消"));
    }
    let mut child = command
        .spawn()
        .map_err(|_| AppError::new("ASR_START_FAILED", "sherpa-onnx组件无法启动"))?;
    let start = Instant::now();
    loop {
        let overflow = std::fs::metadata(&output).is_ok_and(|m| m.len() > 4 * 1024 * 1024);
        if cancel.load(Ordering::SeqCst) || start.elapsed() > Duration::from_secs(1200) || overflow
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new(
                if overflow {
                    "ASR_INVALID_RESULT"
                } else if cancel.load(Ordering::SeqCst) {
                    "QUERY_CANCELED"
                } else {
                    "ASR_TIMEOUT"
                },
                "转写已取消、超时或结果超过限制",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(AppError::new(
                        "ASR_FAILED",
                        "转写失败，请检查运行时与模型是否匹配",
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
    if provider == "cuda" && !cuda_profile_used(&temp.0)? {
        return Err(AppError::new(
            "ASR_CUDA_NOT_ACTIVE",
            "未检测到实际 CUDA 计算；请检查配套组件和驱动，或在设置中选择 CPU",
        ));
    }
    let bytes = zeroize::Zeroizing::new(std::fs::read(output).map_err(|_| AppError::internal())?);
    parse_result(&bytes, id, language, info.duration_ms)
}
fn cuda_profile_used(directory: &Path) -> AppResult<bool> {
    for entry in std::fs::read_dir(directory).map_err(|_| AppError::internal())? {
        let entry = entry.map_err(|_| AppError::internal())?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("execution")
            || entry.path().extension().is_none_or(|s| s != "json")
        {
            continue;
        }
        let metadata = entry.metadata().map_err(|_| AppError::internal())?;
        if metadata.len() > 64 * 1024 * 1024 {
            return Err(AppError::new("ASR_INVALID_RESULT", "CUDA 执行记录超过限制"));
        }
        let profile: serde_json::Value = serde_json::from_reader(
            std::fs::File::open(entry.path()).map_err(|_| AppError::internal())?,
        )
        .map_err(|_| AppError::new("ASR_INVALID_RESULT", "CUDA 执行记录无法解析"))?;
        if profile.as_array().is_some_and(|events| {
            events
                .iter()
                .any(|v| v["cat"] == "Node" && v["args"]["provider"] == "CUDAExecutionProvider")
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}
fn parse_result(bytes: &[u8], id: &str, language: &str, duration: u64) -> AppResult<Transcript> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(invalid("转写结果过大"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| invalid("转写结果无法解析"))?;
    let text = value["text"]
        .as_str()
        .ok_or_else(|| invalid("转写结果缺少文字"))?
        .trim()
        .to_owned();
    if text.len() > 1024 * 1024 || text.contains('\0') {
        return Err(invalid("转写结果文字无效"));
    }
    // Most engines do not provide sentence timing. One whole-audio segment is explicit.
    Ok(Transcript {
        model: id.into(),
        language: value["lang"]
            .as_str()
            .filter(|s| s.len() <= 32)
            .unwrap_or(language)
            .into(),
        segments: if text.is_empty() {
            vec![]
        } else {
            vec![Segment {
                from_ms: 0,
                until_ms: duration,
                text: text.clone(),
            }]
        },
        text,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).unwrap();
            let path =
                std::env::temp_dir().join(format!("werc-sherpa-owned-{}", hex::encode(nonce)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"owned fixture").unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn cuda_profile_requires_executed_cuda_nodes() {
        let fixture = Fixture::new();
        assert!(!cuda_profile_used(&fixture.0).unwrap());
        let profile = fixture.0.join("execution_test.json");
        std::fs::write(&profile, br#"[{"cat":"Node","args":{"provider":"CPUExecutionProvider"}},{"cat":"Session","args":{"provider":"CUDAExecutionProvider"}}]"#).unwrap();
        assert!(!cuda_profile_used(&fixture.0).unwrap());
        std::fs::write(&profile, br#"[{"cat":"Node","args":{"provider":"CUDAExecutionProvider"}}]"#).unwrap();
        assert!(cuda_profile_used(&fixture.0).unwrap());
        std::fs::write(&profile, b"invalid").unwrap();
        assert!(cuda_profile_used(&fixture.0).is_err());
    }
    #[test]
    fn structured_result_does_not_invent_word_times() {
        let r = parse_result(
            br#"{"text":"owned transcript","timestamps":[0.5]}"#,
            "sherpa-paraformer",
            "zh",
            1200,
        )
        .unwrap();
        assert_eq!(r.segments.len(), 1);
        assert_eq!(r.segments[0].until_ms, 1200);
        assert!(parse_result(b"not json", "sherpa-zipformer", "zh", 1).is_err());
    }
    #[test]
    fn zipformer_prefers_quantized_but_rejects_ambiguous_bundles() {
        let f = Fixture::new();
        for p in [
            "encoder-epoch-1.onnx",
            "encoder-epoch-1.int8.onnx",
            "decoder-epoch-1.onnx",
            "joiner-epoch-1.onnx",
            "tokens.txt",
            "weights/arbitrary-external-tensor",
        ] {
            f.file(p);
        }
        let c = configuration("sherpa-zipformer", &f.0).unwrap();
        assert_eq!(c["encoder"], Path::new("encoder-epoch-1.int8.onnx"));
        assert!(
            bundle_files(&f.0)
                .unwrap()
                .contains(&PathBuf::from("weights/arbitrary-external-tensor"))
        );
        f.file("other/encoder-epoch-2.int8.onnx");
        assert!(configuration("sherpa-zipformer", &f.0).is_err());
    }
    #[test]
    fn qwen_and_funasr_require_their_own_complete_tokenizers() {
        let f = Fixture::new();
        for p in [
            "conv_frontend.onnx",
            "encoder.onnx",
            "decoder.onnx",
            "tokenizer/vocab.json",
            "tokenizer/merges.txt",
        ] {
            f.file(p);
        }
        assert!(configuration("sherpa-qwen3-0.6b", &f.0).is_err());
        f.file("tokenizer/tokenizer_config.json");
        assert_eq!(
            configuration("sherpa-qwen3-1.7b", &f.0).unwrap()["qwen3-asr-tokenizer"],
            Path::new("tokenizer")
        );
        for p in ["encoder_adaptor.onnx", "llm.onnx", "embedding.onnx"] {
            f.file(p);
        }
        assert!(configuration("sherpa-funasr-nano", &f.0).is_err());
        f.file("tokenizer/tokenizer.json");
        assert!(configuration("sherpa-funasr-nano", &f.0).is_ok());
    }
}
