//! Explicit local sherpa bundles, sharing the model job and cancellation controls.
use super::*;
use werc_wechat_win::{asr, sherpa_asr};
#[tauri::command]
pub(crate) fn sherpa_catalog() -> AppResult<sherpa_asr::Catalog> {
    sherpa_asr::catalog()
}
#[tauri::command]
pub(crate) fn import_sherpa_bundle(
    state: State<'_, Models>,
    model: String,
    path: String,
) -> AppResult<Progress> {
    if !sherpa_asr::is_runtime(&model) && !sherpa_asr::is_model(&model) {
        return Err(AppError::new("INVALID_ARGUMENT", "未知引擎模型"));
    }
    if path.trim().is_empty() {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "请选择已解压的组件或模型目录",
        ));
    }
    let mut guard = state.0.lock().map_err(|_| AppError::internal())?;
    if guard.as_ref().is_some_and(active) {
        return Err(AppError::new("BUSY", "请先完成或取消当前模型安装"));
    }
    let progress = Progress {
        id: uuid::Uuid::new_v4().to_string(),
        model: format!("sherpa/{model}"),
        phase: "starting".into(),
        bytes: 0,
        total: 0,
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
    tauri::async_runtime::spawn_blocking(move || {
        let result = asr::with_model_maintenance(|| {
            copy_bundle(&state, &id, &model, &PathBuf::from(path), &cancel)
        });
        let bytes = state
            .0
            .lock()
            .ok()
            .and_then(|v| v.as_ref().map(|j| j.progress.bytes))
            .unwrap_or(0);
        match result {
            Ok(()) => update(&state, &id, "complete", bytes, None),
            Err(e) => update(
                &state,
                &id,
                if e.code == "QUERY_CANCELED" {
                    "canceled"
                } else {
                    "failed"
                },
                bytes,
                Some(e),
            ),
        }
    });
    Ok(progress)
}
fn copy_bundle(
    state: &Models,
    id: &str,
    model: &str,
    source: &std::path::Path,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> AppResult<()> {
    let canceled = || {
        if *cancel.borrow() {
            Err(AppError::new("QUERY_CANCELED", "组件导入已取消"))
        } else {
            Ok(())
        }
    };
    canceled()?;
    let destination = if sherpa_asr::is_runtime(model) {
        sherpa_asr::import_runtime_directory(model)?
    } else {
        sherpa_asr::configuration(model, source)?;
        sherpa_asr::model_directory(model)?
    };
    copy_bundle_into(state, id, model, source, &destination, cancel)
}
fn copy_bundle_into(
    state: &Models,
    id: &str,
    model: &str,
    source: &std::path::Path,
    destination: &std::path::Path,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> AppResult<()> {
    let canceled = || {
        if *cancel.borrow() {
            Err(AppError::new("QUERY_CANCELED", "组件导入已取消"))
        } else {
            Ok(())
        }
    };
    canceled()?;
    let files = sherpa_asr::bundle_files(source)?;
    if destination.exists() {
        return Err(AppError::new(
            "MODEL_EXISTS",
            "组件目录已存在；请使用新模型目录或移除原组件后再导入",
        ));
    }
    let chosen = files
        .into_iter()
        .filter(|p| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if sherpa_asr::is_runtime(model) {
                p.components().count() == 1
                    && (name == "sherpa-onnx-offline.exe"
                        || p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"))
                        || name.to_ascii_lowercase().starts_with("license"))
            }
            // ONNX external tensor filenames are arbitrary. Preserve the complete
            // selected model bundle, including tokenizer and license files.
            else {
                true
            }
        })
        .collect::<Vec<_>>();
    if sherpa_asr::is_runtime(model)
        && !chosen
            .iter()
            .any(|p| p == std::path::Path::new("sherpa-onnx-offline.exe"))
    {
        return Err(AppError::new(
            "ASR_MODEL_INVALID",
            "请选择直接包含sherpa-onnx-offline.exe和DLL的目录",
        ));
    }
    let total = chosen
        .iter()
        .try_fold(0u64, |sum, path| {
            std::fs::metadata(source.join(path))
                .ok()
                .and_then(|m| sum.checked_add(m.len()))
        })
        .ok_or_else(AppError::internal)?;
    if total == 0 || total > 24 * 1024 * 1024 * 1024 {
        return Err(AppError::new(
            "MODEL_QUOTA_EXCEEDED",
            "模型目录为空或超过24GiB",
        ));
    }
    if let Some(job) = state
        .0
        .lock()
        .map_err(|_| AppError::internal())?
        .as_mut()
        .filter(|j| j.progress.id == id)
    {
        job.progress.total = total;
    }
    let parent = destination.parent().ok_or_else(AppError::internal)?;
    std::fs::create_dir_all(parent)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法创建组件目录"))?;
    if sherpa_asr::reparse(&std::fs::symlink_metadata(parent).map_err(|_| AppError::internal())?) {
        return Err(AppError::new("MODEL_IO_FAILED", "组件目录不能使用链接"));
    }
    struct Partial(PathBuf);
    impl Drop for Partial {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temporary = Partial(parent.join(format!(".sherpa-{id}.partial")));
    std::fs::create_dir(&temporary.0)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法创建组件临时目录"))?;
    let mut bytes = 0u64;
    let mut manifest = Vec::new();
    for path in chosen {
        canceled()?;
        let from = source.join(&path);
        let before = std::fs::symlink_metadata(&from).map_err(|_| AppError::internal())?;
        if !before.is_file() || sherpa_asr::reparse(&before) {
            return Err(AppError::new("MODEL_IO_FAILED", "导入来源已变化"));
        }
        let output = temporary.0.join(&path);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|_| AppError::internal())?;
        }
        let mut input = std::fs::File::open(&from)
            .map_err(|_| AppError::new("MODEL_IO_FAILED", "模型文件无法读取"))?;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法写入模型文件"))?;
        let mut digest = Sha256::new();
        let mut count = 0u64;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            canceled()?;
            let n = input
                .read(&mut buffer)
                .map_err(|_| AppError::new("MODEL_IO_FAILED", "模型文件读取失败"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n]).map_err(|_| {
                AppError::new("MODEL_IO_FAILED", "模型文件写入失败，请检查剩余空间")
            })?;
            digest.update(&buffer[..n]);
            count += n as u64;
            bytes += n as u64;
            if count > before.len() || bytes > total {
                return Err(AppError::new("MODEL_CHANGED", "模型文件读取期间发生变化"));
            }
            update(state, id, "importing", bytes, None);
        }
        file.sync_all()
            .map_err(|_| AppError::new("MODEL_IO_FAILED", "组件写盘失败"))?;
        drop(file);
        let after = input.metadata().map_err(|_| AppError::internal())?;
        if count != before.len()
            || before.modified().ok() != after.modified().ok()
            || after.len() != before.len()
        {
            return Err(AppError::new("MODEL_CHANGED", "模型文件读取期间发生变化"));
        }
        // Read back destination bytes, rather than treating a successful copy as verification.
        let expected = hex::encode(digest.finalize());
        let mut copy = std::fs::File::open(&output).map_err(|_| AppError::internal())?;
        let mut actual = Sha256::new();
        loop {
            canceled()?;
            let n = copy.read(&mut buffer).map_err(|_| AppError::internal())?;
            if n == 0 {
                break;
            }
            actual.update(&buffer[..n]);
        }
        if hex::encode(actual.finalize()) != expected {
            return Err(AppError::new("MODEL_HASH_MISMATCH", "组件复制摘要不一致"));
        }
        manifest.push(serde_json::json!({"path":path.to_string_lossy().replace('\\',"/"),"size":count,"sha256":expected}));
    }
    canceled()?;
    update(state, id, "verifying", bytes, None);
    if !sherpa_asr::is_runtime(model) {
        sherpa_asr::configuration(model, &temporary.0)?;
    }
    let manifest = serde_json::to_vec_pretty(
        &serde_json::json!({"format":"werc-sherpa-local-import-1","model":model,"files":manifest}),
    )
    .map_err(|_| AppError::internal())?;
    std::fs::write(temporary.0.join("werc-import.json"), manifest)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法保存组件清单"))?;
    canceled()?;
    std::fs::rename(&temporary.0, &destination)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "组件发布失败；原有目录没有覆盖"))?;
    Ok(())
}
#[tauri::command]
pub(crate) fn remove_sherpa_bundle(state: State<'_, Models>, model: String) -> AppResult<()> {
    state.while_idle(|| {
        asr::with_model_maintenance(|| {
            let directory = if sherpa_asr::is_runtime(&model) {
                sherpa_asr::import_runtime_directory(&model)?
            } else {
                sherpa_asr::model_directory(&model)?
            };
            remove_imported(&directory, &model)
        })
    })
}
fn remove_imported(directory: &std::path::Path, model: &str) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(directory)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "组件目录不存在"))?;
    if !metadata.is_dir() || sherpa_asr::reparse(&metadata) {
        return Err(AppError::new("MODEL_IO_FAILED", "不能移除链接目录"));
    }
    let bytes = std::fs::read(directory.join("werc-import.json"))
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "仅可移除通过此面板导入的组件"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "组件导入清单无效"))?;
    if manifest["format"] != "werc-sherpa-local-import-1" || manifest["model"] != model {
        return Err(AppError::new("MODEL_IO_FAILED", "组件导入清单不匹配"));
    }
    // Validate the resolved tree before recursive removal. Import roots are
    // fixed by the known model ID, never supplied by the renderer.
    sherpa_asr::bundle_files(directory)?;
    std::fs::remove_dir_all(directory)
        .map_err(|_| AppError::new("MODEL_IO_FAILED", "无法移除组件，请检查文件是否被占用"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Owned(PathBuf);
    impl Owned {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("werc-owned-sherpa-import-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn local_bundle_preserves_external_tensors_and_has_owned_removal() {
        let f = Owned::new();
        let source = f.0.join("source");
        std::fs::create_dir_all(source.join("weights")).unwrap();
        for name in ["model.onnx", "tokens.txt", "weights/arbitrary-tensor-file"] {
            std::fs::write(source.join(name), b"owned bundle").unwrap();
        }
        let dest = f.0.join("installed");
        let state = Models::default();
        let (_, cancel) = tokio::sync::watch::channel(false);
        copy_bundle_into(
            &state,
            "owned",
            "sherpa-paraformer",
            &source,
            &dest,
            &cancel,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dest.join("weights/arbitrary-tensor-file")).unwrap(),
            b"owned bundle"
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dest.join("werc-import.json")).unwrap()).unwrap();
        assert_eq!(manifest["files"].as_array().unwrap().len(), 3);
        assert_eq!(
            copy_bundle_into(
                &state,
                "another",
                "sherpa-paraformer",
                &source,
                &dest,
                &cancel
            )
            .err()
            .unwrap()
            .code,
            "MODEL_EXISTS"
        );
        assert!(remove_imported(&source, "sherpa-paraformer").is_err());
        assert!(remove_imported(&dest, "runtime").is_err());
        remove_imported(&dest, "sherpa-paraformer").unwrap();
        assert!(!dest.exists());
        assert!(source.exists());
    }
    #[test]
    fn canceled_and_incomplete_bundle_do_not_publish_destination() {
        let f = Owned::new();
        let source = f.0.join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("model.onnx"), b"owned").unwrap();
        let state = Models::default();
        let dest = f.0.join("installed");
        let (_, cancel) = tokio::sync::watch::channel(true);
        assert_eq!(
            copy_bundle_into(
                &state,
                "owned",
                "sherpa-paraformer",
                &source,
                &dest,
                &cancel
            )
            .err()
            .unwrap()
            .code,
            "QUERY_CANCELED"
        );
        let (_, cancel) = tokio::sync::watch::channel(false);
        assert!(
            copy_bundle_into(
                &state,
                "owned",
                "sherpa-paraformer",
                &source,
                &dest,
                &cancel
            )
            .is_err()
        );
        assert!(!dest.exists());
    }
}
