//! Explicit optional NVIDIA setup. Detection is local; downloads require a separate action.
use super::{models::*, *};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use werc_domain::AppResult;
const RUNTIME_FILES: &[&str] = &[
    "sherpa-onnx-offline.exe", "onnxruntime.dll", "onnxruntime_providers_shared.dll",
    "onnxruntime_providers_cuda.dll", "cudnn64_9.dll", "cudart64_12.dll",
    "cublas64_12.dll", "cublasLt64_12.dll",
];
fn runtime_ready(directory: &Path) -> bool {
    RUNTIME_FILES.iter().all(|name| directory.join(name).is_file())
}
#[derive(Clone, Deserialize)]
struct Package {
    id: String,
    format: String,
    size: u64,
    sha256: String,
    url: String,
}
#[derive(Deserialize)]
struct Registry {
    id: String,
    minimum_windows_driver: String,
    packages: Vec<Package>,
}
fn registry() -> AppResult<Registry> {
    serde_json::from_str(include_str!("../../../../native/asr/CUDA.json"))
        .map_err(|_| AppError::internal())
}
#[derive(Serialize)]
pub(super) struct Detection {
    available: bool,
    reason: String,
    devices: Vec<Device>,
    download_bytes: u64,
    plan: String,
    installed: bool,
}
use werc_wechat_win::cuda_devices::Device;
pub(super) fn detect() -> AppResult<Detection> {
    let registry = registry()?;
    let supported_platform = cfg!(all(windows, target_arch = "x86_64"));
    let devices = werc_wechat_win::cuda_devices::detect(&registry.minimum_windows_driver)?;
    let available = supported_platform && devices.iter().any(|device| device.compatible);
    let runtime = werc_wechat_win::sherpa_asr::import_runtime_directory("runtime-cuda")?;
    let installed = runtime.join("werc-cuda-install.json").is_file()
        && runtime_ready(&runtime);
    Ok(Detection {
        available,
        reason: if !supported_platform {
            "此自动安装方案仅支持 Windows x64；当前平台可继续使用 CPU".into()
        } else if available {
            "可安装 CUDA 12.8 / cuDNN 9.8 配套识别组件；不会修改显卡驱动".into()
        } else if devices.is_empty() {
            "未检测到可用的 NVIDIA CUDA 设备；AMD、Intel 和无独显电脑可继续使用 CPU".into()
        } else {
            format!("当前设备不满足此下载方案，需要 NVIDIA 驱动 {} 或更新且为 Turing 或更新架构；可继续使用 CPU", registry.minimum_windows_driver)
        },
        devices,
        download_bytes: registry.packages.iter().map(|p| p.size).sum(),
        plan: registry.id,
        installed,
    })
}
#[tauri::command]
pub(super) async fn cuda_detect() -> AppResult<Detection> {
    tauri::async_runtime::spawn_blocking(detect)
        .await
        .map_err(|_| AppError::internal())?
}
pub(super) fn require_ready(model_directory: &Path)->AppResult<()>{
    let detected=detect()?;
    if !detected.available{return Err(AppError::new("GPU_UNSUPPORTED",&detected.reason));}
    let local=model_directory.join(".runtimes/sherpa-cuda");
    let resources=werc_wechat_win::asr::root()?;
    let sherpa=if local.is_dir(){local}else{resources.join("sherpa-cuda")};
    let whisper=resources.join("cuda/whisper-cli.exe");
    if !runtime_ready(&sherpa)&&!whisper.is_file(){return Err(AppError::new("ASR_CUDA_NOT_READY","所选模型目录尚无完整 CUDA 配套组件，请先使用 CPU 并完成安装；已有设置保留"));}
    Ok(())
}
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn canceled(cancel: &tokio::sync::watch::Receiver<bool>) -> AppResult<()> {
    if *cancel.borrow() {
        Err(AppError::new("QUERY_CANCELED", "CUDA 安装已取消"))
    } else {
        Ok(())
    }
}
fn selected(package: &str, path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    if package == "paraformer" {
        return matches!(name, "model.int8.onnx" | "model.onnx" | "tokens.txt")
            || lower.starts_with("license");
    }
    if lower.starts_with("license") || lower.contains("eula") {
        return true;
    }
    if package == "sherpa" {
        path.parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "bin")
            && (name == "sherpa-onnx-offline.exe"
                || lower.ends_with(".dll") && !lower.contains("tensorrt"))
    } else {
        lower.ends_with(".dll")
            && match package {
                "cudart" => lower.starts_with("cudart64_"),
                "cublas" => lower.starts_with("cublas"),
                "cudnn" => lower.starts_with("cudnn"),
                _ => false,
            }
    }
}
fn extract(
    package: &Package,
    archive: &Path,
    into: &Path,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> AppResult<()> {
    let copy = |path: PathBuf, input: &mut dyn Read, size: u64| -> AppResult<()> {
        canceled(cancel)?;
        if !selected(&package.id, &path) {
            return Ok(());
        }
        if size > 2 * 1024 * 1024 * 1024 {
            return Err(AppError::new("GPU_PACKAGE_INVALID", "组件文件大小异常"));
        }
        let name = path.file_name().ok_or_else(AppError::internal)?;
        let is_dll = path
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("dll"));
        let output = if is_dll || name == "sherpa-onnx-offline.exe" || package.id == "paraformer" {
            into.join(name)
        } else {
            let dir = into.join("licenses").join(&package.id);
            std::fs::create_dir_all(&dir).map_err(|_| AppError::internal())?;
            dir.join(name)
        };
        // An archive may repeat its license in several folders. Runtime files must be unique.
        if output.exists() && !is_dll && name != "sherpa-onnx-offline.exe" {
            return Ok(());
        }
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)
            .map_err(|_| AppError::new("GPU_PACKAGE_INVALID", "组件文件重复或无法写入"))?;
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            canceled(cancel)?;
            let n = input
                .read(&mut buffer)
                .map_err(|_| AppError::new("GPU_PACKAGE_INVALID", "无法解压组件"))?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > size {
                return Err(AppError::new("GPU_PACKAGE_INVALID", "组件解压大小不符"));
            }
            file.write_all(&buffer[..n])
                .map_err(|_| AppError::new("MODEL_IO_FAILED", "组件写盘失败，请检查剩余空间"))?;
        }
        if count != size {
            return Err(AppError::new("GPU_PACKAGE_INVALID", "组件文件不完整"));
        }
        file.sync_all().map_err(|_| AppError::internal())?;
        Ok(())
    };
    let file = std::fs::File::open(archive).map_err(|_| AppError::internal())?;
    if package.format == "tar.bz2" {
        let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(file));
        for entry in tar
            .entries()
            .map_err(|_| AppError::new("GPU_PACKAGE_INVALID", "组件归档无效"))?
        {
            let mut entry =
                entry.map_err(|_| AppError::new("GPU_PACKAGE_INVALID", "组件归档损坏"))?;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let path = entry.path().map_err(|_| AppError::internal())?.into_owned();
            let size = entry.size();
            copy(path, &mut entry, size)?;
        }
    } else {
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|_| AppError::new("GPU_PACKAGE_INVALID", "组件 ZIP 无效"))?;
        for n in 0..zip.len() {
            let mut entry = zip.by_index(n).map_err(|_| AppError::internal())?;
            if entry.is_dir() {
                continue;
            }
            let path = entry
                .enclosed_name()
                .ok_or_else(|| AppError::new("GPU_PACKAGE_INVALID", "组件路径无效"))?;
            let size = entry.size();
            copy(path, &mut entry, size)?;
        }
    }
    Ok(())
}
async fn install(
    state: Models,
    id: String,
    registry: Registry,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> AppResult<()> {
    let destination = werc_wechat_win::sherpa_asr::import_runtime_directory("runtime-cuda")?;
    if destination.exists() {
        return Err(AppError::new(
            "MODEL_EXISTS",
            "CUDA 组件目录已存在；可直接使用，或在模型管理中移除后重新安装",
        ));
    }
    let parent = destination.parent().ok_or_else(AppError::internal)?;
    std::fs::create_dir_all(parent)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法创建 CUDA 组件目录"))?;
    let staging = Staging(parent.join(format!(".cuda-{id}.partial")));
    std::fs::create_dir(&staging.0).map_err(|_| AppError::internal())?;
    let runtime = staging.0.join("runtime");
    let default_model = staging.0.join("default-model");
    std::fs::create_dir(&default_model).map_err(|_| AppError::internal())?;
    std::fs::create_dir(&runtime).map_err(|_| AppError::internal())?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| AppError::internal())?;
    let mut total = 0u64;
    for package in &registry.packages {
        canceled(&cancel)?;
        let mut response = tokio::select! {_ = cancel.changed()=>return Err(AppError::new("QUERY_CANCELED","CUDA 下载已取消")),response=client.get(&package.url).send()=>response.map_err(|_|AppError::new("MODEL_NETWORK_FAILED","CUDA 组件连接失败，请稍后重试"))?.error_for_status().map_err(|_|AppError::new("MODEL_NETWORK_FAILED","CUDA 下载源暂不可用"))?};
        let archive = staging.0.join(&package.id);
        let mut file = std::fs::File::create(&archive).map_err(|_| AppError::internal())?;
        let mut hash = Sha256::new();
        let mut size = 0u64;
        loop {
            canceled(&cancel)?;
            let chunk = tokio::select! {_ = cancel.changed()=>return Err(AppError::new("QUERY_CANCELED","CUDA 下载已取消")),r=response.chunk()=>r.map_err(|_|AppError::new("MODEL_NETWORK_FAILED","CUDA 下载中断，可稍后重新安装"))?};
            let Some(chunk) = chunk else {
                break;
            };
            size += chunk.len() as u64;
            if size > package.size {
                return Err(AppError::new("MODEL_HASH_MISMATCH", "CUDA 下载大小异常"));
            }
            file.write_all(&chunk).map_err(|_| {
                AppError::new("MODEL_IO_FAILED", "CUDA 下载写盘失败，请检查剩余空间")
            })?;
            hash.update(&chunk);
            update(&state, &id, "downloading", total + size, None);
        }
        file.sync_all().map_err(|_| AppError::internal())?;
        drop(file);
        if size != package.size || hex::encode(hash.finalize()) != package.sha256 {
            return Err(AppError::new(
                "MODEL_HASH_MISMATCH",
                "CUDA 组件摘要不匹配，未安装",
            ));
        }
        total += size;
        update(&state, &id, "verifying", total, None);
        // Decompression is blocking; run it off the async executor and retain cancellation.
        let spec = package.clone();
        let input = archive.clone();
        let into = if package.id == "paraformer" {
            default_model.clone()
        } else {
            runtime.clone()
        };
        let token = cancel.clone();
        tauri::async_runtime::spawn_blocking(move || extract(&spec, &input, &into, &token))
            .await
            .map_err(|_| AppError::internal())??;
        std::fs::remove_file(archive).map_err(|_| AppError::internal())?;
    }
    for name in RUNTIME_FILES {
        if !runtime.join(name).is_file() {
            return Err(AppError::new(
                "GPU_PACKAGE_INVALID",
                "下载组件缺少必要运行文件，未安装",
            ));
        }
    }
    let manifest=serde_json::to_vec_pretty(&serde_json::json!({"format":"werc-sherpa-local-import-1","model":"runtime-cuda","plan":registry.id})).map_err(|_|AppError::internal())?;
    std::fs::write(runtime.join("werc-import.json"), &manifest)
        .map_err(|_| AppError::internal())?;
    std::fs::write(runtime.join("werc-cuda-install.json"), &manifest)
        .map_err(|_| AppError::internal())?;
    werc_wechat_win::sherpa_asr::configuration("sherpa-paraformer", &default_model)?;
    std::fs::write(
        default_model.join("werc-import.json"),
        serde_json::to_vec(
            &serde_json::json!({"format":"werc-sherpa-local-import-1","model":"sherpa-paraformer"}),
        )
        .map_err(|_| AppError::internal())?,
    )
    .map_err(|_| AppError::internal())?;
    let model_destination = werc_wechat_win::sherpa_asr::model_directory("sherpa-paraformer")?;
    canceled(&cancel)?;
    werc_wechat_win::asr::with_model_maintenance(|| {
        let new_model = !model_destination.exists();
        if new_model {
            std::fs::rename(&default_model, &model_destination)
                .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法安装默认识别模型"))?;
        }
        let result = std::fs::rename(&runtime, &destination)
            .map_err(|_| AppError::new("MODEL_IO_FAILED", "CUDA 安装发布失败；原有组件保留"));
        if result.is_err() && new_model {
            let _ = std::fs::rename(&model_destination, &default_model);
        }
        result
    })?;
    Ok(())
}
#[tauri::command]
pub(super) async fn install_cuda_runtime(
    handle: tauri::AppHandle,
    state: State<'_, Models>,
) -> AppResult<Progress> {
    let detected = cuda_detect().await?;
    if !detected.available {
        return Err(AppError::new("GPU_UNSUPPORTED", &detected.reason));
    }
    let registry = registry()?;
    let mut guard = state.0.lock().map_err(|_| AppError::internal())?;
    if guard.as_ref().is_some_and(active) {
        return Err(AppError::new("MODEL_BUSY", "请先完成或取消当前组件安装"));
    }
    let progress = Progress {
        id: uuid::Uuid::new_v4().to_string(),
        model: "CUDA 识别组件".into(),
        phase: "starting".into(),
        bytes: 0,
        total: registry.packages.iter().map(|p| p.size).sum(),
        error: None,
    };
    let (sender, cancel) = tokio::sync::watch::channel(false);
    *guard = Some(Job {
        progress: progress.clone(),
        cancel: sender,
    });
    drop(guard);
    let state = state.inner().clone();
    let id = progress.id.clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let result = install(state.clone(), id.clone(), registry, cancel).await;
        let bytes = state
            .0
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|j| j.progress.bytes))
            .unwrap_or(0);
        match result {
            Ok(()) => {
                update(&state, &id, "complete", bytes, None);
                diagnostics::event(
                    &handle,
                    "asr",
                    "CUDA_COMPONENTS_INSTALLED",
                    Some(started.elapsed()),
                );
            }
            Err(e) => {
                diagnostics::record(&handle, "asr", &e);
                update(
                    &state,
                    &id,
                    if e.code == "QUERY_CANCELED" {
                        "canceled"
                    } else {
                        "failed"
                    },
                    bytes,
                    Some(e),
                );
            }
        }
    });
    Ok(progress)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_do_not_assign_cuda_to_unknown_or_old_devices() {
        use werc_wechat_win::cuda_devices::compatible;
        assert!(compatible("581.15", "12.0", "570.65"));
        assert!(!compatible("560.94", "8.6", "570.65"));
        assert!(!compatible("581.15", "6.1", "570.65"));
        assert!(!compatible("N/A", "N/A", "570.65"));
        let r = registry().unwrap();
        assert_eq!(r.packages.len(), 5);
        assert!(
            r.packages
                .iter()
                .all(|p| p.url.starts_with("https://") && p.sha256.len() == 64)
        );
        assert!(!selected("sherpa", Path::new("root/lib/onnxruntime.dll")));
        assert!(selected("sherpa", Path::new("root/bin/onnxruntime.dll")));
    }
    #[tokio::test]
    #[ignore = "explicit real NVIDIA/network verification against isolated workspace data"]
    async fn cuda_real_install_and_transcribe() {
        use werc_wechat_win::{asr, sherpa_asr};
        let project = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .unwrap();
        let root = project.join("target/cuda-verification/owned-models");
        std::fs::create_dir_all(&root).unwrap();
        asr::configure_models_execution_with(root.clone(), "cuda", 2, || Ok(())).unwrap();
        let detected = detect().unwrap();
        assert!(detected.available, "{}", detected.reason);
        let state = Models::default();
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        let destination = sherpa_asr::import_runtime_directory("runtime-cuda").unwrap();
        if !destination.exists() {
            install(
                state,
                "real-cuda-install".into(),
                registry().unwrap(),
                cancel,
            )
            .await
            .unwrap();
        }
        assert!(
            sherpa_asr::configuration("sherpa-paraformer", &root.join("sherpa-paraformer")).is_ok()
        );
        let wav = std::fs::read(
            project.join("target/voice-validation-20261111/automation-test-native-voice.wav"),
        )
        .unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let before = asr::cache_identity("sherpa-paraformer", &cancel).unwrap();
        let started = Instant::now();
        let result = asr::transcribe(&wav, "sherpa-paraformer", "zh", &cancel).unwrap();
        assert!(!result.text.trim().is_empty());
        let elapsed = started.elapsed().as_millis();
        let after = asr::cache_identity("sherpa-paraformer", &cancel).unwrap();
        assert_eq!(before, after);
        std::fs::write(project.join("target/cuda-verification/result.json"),serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"detection":detected,"plan":registry().unwrap().id,"cuda_execution_profile_verified":true,"duration_ms":elapsed,"text":result.text,"cache_identity_stable":true,"runtime_directory_relative":"target/cuda-verification/owned-models/.runtimes/sherpa-cuda"})).unwrap()).unwrap();
    }
}
