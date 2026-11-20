//! Explicit model installation. Audio is never part of a network request.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use werc_domain::AppResult;
#[path="models_sherpa.rs"]
pub(crate) mod sherpa;

#[derive(Clone, Deserialize, Serialize)]
struct Model {
    id: String,
    file: String,
    sha256: String,
    size: u64,
}
#[derive(Deserialize)]
struct Registry {
    revision: String,
    models: Vec<Model>,
}
fn registry() -> AppResult<Registry> {
    serde_json::from_str(include_str!("../../../../native/asr/MODELS.json"))
        .map_err(|_| AppError::internal())
}
#[derive(Clone, Serialize)]
pub(super) struct Progress {
    pub(super) id: String,
    pub(super) model: String,
    pub(super) phase: String,
    pub(super) bytes: u64,
    pub(super) total: u64,
    pub(super) error: Option<AppError>,
}
pub(super) struct Job {
    pub(super) progress: Progress,
    pub(super) cancel: tokio::sync::watch::Sender<bool>,
}
#[derive(Default, Clone)]
pub(super) struct Models(pub(super) Arc<Mutex<Option<Job>>>);
#[derive(Serialize)]
pub(super) struct Catalog {
    models: Vec<Model>,
    installed: Vec<String>,
    removable: Vec<String>,
    engine_available: bool,
    directory: String,
    job: Option<Progress>,
}
#[tauri::command]
pub(super) fn model_catalog(state: State<'_, Models>) -> AppResult<Catalog> {
    let status = werc_wechat_win::asr::status()?;
    Ok(Catalog {
        models: registry()?.models,
        removable: status.models.iter().filter(|id| {
            werc_wechat_win::asr::model_directory().is_ok_and(|root| root.join(format!("ggml-{id}.bin")).is_file())
        }).cloned().collect(),
        installed: status.models,
        engine_available: werc_wechat_win::asr::whisper_runtime()?
            .join("whisper-cli.exe")
            .is_file(),
        directory: werc_wechat_win::asr::model_directory()?
            .to_string_lossy()
            .into_owned(),
        job: state
            .0
            .lock()
            .map_err(|_| AppError::internal())?
            .as_ref()
            .map(|j| j.progress.clone()),
    })
}
impl Models {
    pub(super) fn while_idle<T>(&self, apply: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
        let guard = self.0.lock().map_err(|_| AppError::internal())?;
        if guard.as_ref().is_some_and(active) {
            return Err(AppError::new("MODEL_BUSY", "模型安装期间不能更改目录"));
        }
        apply()
    }
}
pub(super) fn active(job: &Job) -> bool {
    matches!(
        job.progress.phase.as_str(),
        "starting" | "downloading" | "importing" | "verifying"
    )
}
pub(super) fn update(state: &Models, id: &str, phase: &str, bytes: u64, error: Option<AppError>) {
    if let Ok(mut guard) = state.0.lock() {
        if let Some(job) = guard.as_mut().filter(|j| j.progress.id == id) {
            job.progress.phase = phase.into();
            job.progress.bytes = bytes;
            job.progress.error = error;
        }
    }
}
struct Temporary {
    path: PathBuf,
    file: Option<std::fs::File>,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}
fn prepare(directory: &std::path::Path, id: &str) -> AppResult<Temporary> {
    std::fs::create_dir_all(directory)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法创建模型目录"))?;
    let path = directory.join(format!(".{id}.partial"));
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法创建模型临时文件"))?;
    Ok(Temporary {
        path,
        file: Some(file),
    })
}
fn publish(
    mut temp: Temporary,
    model: &Model,
    count: u64,
    hash: Sha256,
    directory: &std::path::Path,
) -> AppResult<()> {
    if count != model.size || hex::encode(hash.finalize()) != model.sha256 {
        return Err(AppError::new(
            "MODEL_HASH_MISMATCH",
            "模型大小或摘要不符，未安装；请重新选择下载源或本地文件",
        ));
    }
    temp.file
        .as_ref()
        .ok_or_else(AppError::internal)?
        .sync_all()
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "模型写盘失败"))?;
    drop(temp.file.take());
    // Publish without replacing an existing model, even if another process installed it.
    export_file::publish_new(&temp.path, &directory.join(&model.file))?;
    Ok(())
}
async fn download(
    state: Models,
    id: String,
    model: Model,
    url: String,
    directory: PathBuf,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> AppResult<()> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(20))
        .read_timeout(std::time::Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .map_err(|_| AppError::new("MODEL_NETWORK_FAILED", "无法初始化模型下载"))?;
    let mut response = tokio::select! {
        _ = cancel.changed() => return Err(AppError::new("QUERY_CANCELED", "模型下载已取消")),
        result = client.get(url).send() => result.map_err(|_| AppError::new("MODEL_NETWORK_FAILED", "模型下载连接失败，可切换下载源"))?
            .error_for_status().map_err(|_| AppError::new("MODEL_NETWORK_FAILED", "下载源拒绝了请求"))?
    };
    let mut temp = prepare(&directory, &id)?;
    let mut count = 0;
    let mut hash = Sha256::new();
    loop {
        if *cancel.borrow() {
            return Err(AppError::new("QUERY_CANCELED", "模型下载已取消"));
        }
        let chunk = tokio::select! {
            _ = cancel.changed() => return Err(AppError::new("QUERY_CANCELED", "模型下载已取消")),
            result = response.chunk() => result.map_err(|_| AppError::new("MODEL_NETWORK_FAILED", "模型下载中断，请重试"))?
        };
        let Some(chunk) = chunk else {
            break;
        };
        count += chunk.len() as u64;
        if count > model.size {
            return Err(AppError::new(
                "MODEL_HASH_MISMATCH",
                "下载文件超过模型预期大小",
            ));
        }
        temp.file
            .as_mut()
            .ok_or_else(AppError::internal)?
            .write_all(&chunk)
            .map_err(|_| AppError::new("MODEL_IO_FAILED", "模型写盘失败，请检查可用空间"))?;
        hash.update(&chunk);
        update(&state, &id, "downloading", count, None);
    }
    if *cancel.borrow() {
        return Err(AppError::new("QUERY_CANCELED", "模型下载已取消"));
    }
    update(&state, &id, "verifying", count, None);
    publish(temp, &model, count, hash, &directory)
}
fn import(
    state: &Models,
    id: &str,
    model: &Model,
    source: &str,
    directory: &std::path::Path,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> AppResult<()> {
    let mut input = std::fs::File::open(source)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "本地模型无法打开"))?;
    if input.metadata().map_err(|_| AppError::internal())?.len() != model.size {
        return Err(AppError::new(
            "MODEL_HASH_MISMATCH",
            "文件大小与所选模型不符",
        ));
    }
    let mut temp = prepare(directory, id)?;
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        if *cancel.borrow() {
            return Err(AppError::new("QUERY_CANCELED", "模型导入已取消"));
        }
        let n = input
            .read(&mut buffer)
            .map_err(|_| AppError::new("MODEL_IO_FAILED", "本地模型读取失败"))?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > model.size {
            return Err(AppError::new(
                "MODEL_HASH_MISMATCH",
                "模型文件在读取时发生变化",
            ));
        }
        temp.file
            .as_mut()
            .ok_or_else(AppError::internal)?
            .write_all(&buffer[..n])
            .map_err(|_| AppError::new("MODEL_IO_FAILED", "模型写盘失败"))?;
        hash.update(&buffer[..n]);
        update(state, id, "importing", count, None);
    }
    if *cancel.borrow() {
        return Err(AppError::new("QUERY_CANCELED", "模型导入已取消"));
    }
    publish(temp, model, count, hash, directory)
}
#[tauri::command]
pub(super) fn install_model(
    state: State<'_, Models>,
    model: String,
    source: String,
    path: Option<String>,
) -> AppResult<Progress> {
    let registry = registry()?;
    let model = registry
        .models
        .into_iter()
        .find(|m| m.id == model)
        .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "未知模型"))?;
    if !matches!(source.as_str(), "official" | "mirror" | "local")
        || (source == "local" && path.as_ref().is_none_or(|p| p.trim().is_empty()))
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "请选择下载源或本地模型文件",
        ));
    }
    let mut guard = state.0.lock().map_err(|_| AppError::internal())?;
    let directory = werc_wechat_win::asr::model_directory()?;
    if guard.as_ref().is_some_and(active) {
        return Err(AppError::new("BUSY", "请先完成或取消当前模型安装"));
    }
    if directory.join(&model.file).exists() {
        return Err(AppError::new("MODEL_EXISTS", "模型已存在，请先移除旧文件"));
    }
    let progress = Progress {
        id: uuid::Uuid::new_v4().to_string(),
        model: model.id.clone(),
        phase: "starting".into(),
        bytes: 0,
        total: model.size,
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
        let result = if source == "local" {
            let worker = state.clone();
            let job = id.clone();
            let spec = model.clone();
            let dir = directory.clone();
            tauri::async_runtime::spawn_blocking(move || {
                import(
                    &worker,
                    &job,
                    &spec,
                    &path.unwrap_or_default(),
                    &dir,
                    &cancel,
                )
            })
            .await
            .map_err(|_| AppError::internal())
            .and_then(|v| v)
        } else {
            let host = if source == "mirror" {
                "hf-mirror.com"
            } else {
                "huggingface.co"
            };
            let url = format!(
                "https://{host}/ggerganov/whisper.cpp/resolve/{}/{}",
                registry.revision, model.file
            );
            download(
                state.clone(),
                id.clone(),
                model.clone(),
                url,
                directory,
                cancel,
            )
            .await
        };
        match result {
            Ok(()) => update(&state, &id, "complete", model.size, None),
            Err(e) => {
                let phase = if e.code == "QUERY_CANCELED" {
                    "canceled"
                } else {
                    "failed"
                };
                let bytes = state
                    .0
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|j| j.progress.bytes))
                    .unwrap_or(0);
                update(&state, &id, phase, bytes, Some(e));
            }
        }
    });
    Ok(progress)
}
#[tauri::command]
pub(super) fn cancel_model_install(state: State<'_, Models>, id: String) -> AppResult<()> {
    if let Some(job) = state
        .0
        .lock()
        .map_err(|_| AppError::internal())?
        .as_ref()
        .filter(|j| j.progress.id == id && active(j))
    {
        let _ = job.cancel.send(true);
    }
    Ok(())
}
#[tauri::command]
pub(super) fn remove_model(state: State<'_, Models>, model: String) -> AppResult<()> {
    let guard = state.0.lock().map_err(|_| AppError::internal())?;
    if guard.as_ref().is_some_and(active) {
        return Err(AppError::new("BUSY", "模型安装中"));
    }
    let model = registry()?
        .models
        .into_iter()
        .find(|m| m.id == model)
        .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "未知模型"))?;
    werc_wechat_win::asr::with_model_maintenance(|| {
        std::fs::remove_file(werc_wechat_win::asr::model_directory()?.join(model.file)).map_err(
            |_| {
                AppError::new(
                    "MODEL_IO_FAILED",
                    "无法移除此模型；随程序附带的模型不在此处删除",
                )
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_model_publication_rejects_corruption_and_never_overwrites() {
        let root =
            std::env::temp_dir().join(format!("werc-model-自动化测试-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let bytes = b"owned-model";
        let model = Model {
            id: "owned".into(),
            file: "ggml-owned.bin".into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
        };
        for valid in [false, true, true] {
            let mut temp = prepare(&root, &uuid::Uuid::new_v4().to_string()).unwrap();
            let content = if valid { bytes.as_slice() } else { b"corrupt" };
            temp.file.as_mut().unwrap().write_all(content).unwrap();
            let mut hash = Sha256::new();
            hash.update(content);
            let already = root.join(&model.file).exists();
            let result = publish(temp, &model, content.len() as u64, hash, &root);
            assert_eq!(result.is_ok(), valid && !already);
            assert_eq!(
                std::fs::read_dir(&root).unwrap().count(),
                usize::from(valid)
            );
        }
        assert_eq!(std::fs::read(root.join(&model.file)).unwrap(), bytes);
        std::fs::remove_file(root.join(&model.file)).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
