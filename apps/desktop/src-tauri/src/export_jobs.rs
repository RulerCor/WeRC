use super::*;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use werc_application::export_plans::*;
use werc_domain::AppResult;

#[tauri::command]
pub(super) async fn export_plans(
    state: State<'_, DesktopState>,
    account_id: String,
    before: Option<i64>,
) -> AppResult<(Vec<PlanItem>, Option<i64>)> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.export_plans_local(&account_id, before))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn save_export_plan(
    state: State<'_, DesktopState>,
    account_id: String,
    id: Option<String>,
    revision: i64,
    spec: ExportPlanSpec,
) -> AppResult<PlanItem> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.save_export_plan_local(&account_id, id.as_deref(), revision, spec)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn export_plan_action(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    revision: i64,
    action: String,
) -> AppResult<()> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || match action.as_str() {
        "delete" => app.delete_export_plan_local(&account_id, &id, revision),
        "run" => app
            .begin_export_run_local(&account_id, &id, true)
            .map(|_| ()),
        "cancel" => app.cancel_export_run_local(&account_id, &id),
        "resume" => app.resume_export_run_local(&account_id, &id).map(|_| ()),
        _ => Err(AppError::new("INVALID_ARGUMENT", "计划操作无效")),
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[derive(Serialize)]
pub(super) struct RunView {
    item: RunItem,
    exists: Vec<bool>,
}
#[tauri::command]
pub(super) async fn export_runs(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    before: Option<i64>,
) -> AppResult<(Vec<RunView>, Option<i64>)> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (items, next) = app.export_runs_local(&account_id, before)?;
        Ok((
            items
                .into_iter()
                .map(|item| {
                    let exists = item
                        .run
                        .artifacts
                        .iter()
                        .map(|a| {
                            export_file::destination(&a.path, &downloads).is_ok_and(|p| p.is_file())
                        })
                        .collect();
                    RunView { item, exists }
                })
                .collect(),
            next,
        ))
    })
    .await
    .map_err(|_| AppError::internal())?
}
struct HashWriter<'a> {
    file: &'a mut std::fs::File,
    hash: Sha256,
    size: u64,
}
impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(bytes)?;
        self.hash.update(&bytes[..n]);
        self.size += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
pub(super) fn matches_artifact(path: &Path, artifact: &ExportArtifact) -> AppResult<bool> {
    let mut file = std::fs::File::open(path)
        .map_err(|_| AppError::new("EXPORT_READ_FAILED", "已登记产物无法打开"))?;
    if file.metadata().map_err(|_| AppError::internal())?.len() != artifact.size {
        return Ok(false);
    }
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| AppError::new("EXPORT_READ_FAILED", "核对产物失败"))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hex::encode(hash.finalize()) == artifact.sha256)
}
pub(super) fn output_path(
    downloads: &Path,
    directory: &str,
    id: &str,
    format: &str,
) -> AppResult<(String, PathBuf)> {
    let folder = Path::new(directory);
    let mut current = downloads.to_owned();
    for part in folder.components() {
        let std::path::Component::Normal(name) = part else {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "计划目录必须是下载目录内的相对路径",
            ));
        };
        let candidate = current.join(name);
        let relative = candidate.to_string_lossy();
        export_file::destination(&relative, downloads)?;
        match std::fs::create_dir(&candidate) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(AppError::new("EXPORT_WRITE_FAILED", "无法创建计划输出目录")),
        }
        current = candidate;
    }
    let extension=werc_application::capture_tools::capture_export_extension(format).ok_or_else(||AppError::new("INVALID_ARGUMENT","导出格式无效"))?;
    let name=if format.contains('_'){format!("WeRC-{id}-{format}.{extension}")}else{format!("WeRC-{id}.{extension}")};
    let relative = folder
        .join(name)
        .to_string_lossy()
        .into_owned();
    Ok((
        relative.clone(),
        export_file::destination(&relative, downloads)?,
    ))
}
fn execute(
    app: &Application,
    account: &str,
    item: RunItem,
    downloads: &Path,
    allow_live: bool,
) -> AppResult<()> {
    let id = &item.id;
    let run = &item.run;
    if run.cancel_requested {
        return Err(AppError::new("QUERY_CANCELED", "任务已取消"));
    }
    if run.spec.refresh_source {
        if !allow_live {
            return Err(AppError::new("FORBIDDEN", "隔离测试不读取真实来源"));
        }
        app.export_run_phase_local(account, id, "reading")?;
        let mut capture = app.fork_source_capture_local(
            account,
            &run.spec.source_capture,
            &run.capture_id,
            run.from,
            run.until,
        )?;
        while !capture.complete_selected_sources {
            let current = app.export_run_local(account, id)?;
            if current.run.cancel_requested {
                return Err(AppError::new("QUERY_CANCELED", "任务已取消"));
            }
            capture = app.step_source_capture_local(account, &run.capture_id, id)?;
        }
    }
    app.export_run_phase_local(account, id, "exporting")?;
    let mut options = run.spec.options.clone();
    options.filter.from = run.from;
    options.filter.until = Some(run.until);
    for format in &run.spec.formats {
        let current = app.export_run_local(account, id)?;
        if current.run.cancel_requested {
            return Err(AppError::new("QUERY_CANCELED", "任务已取消"));
        }
        let (relative, path) = output_path(downloads, &run.spec.directory, id, format)?;
        if path.exists() {
            let old = current
                .run
                .artifacts
                .iter()
                .find(|a| a.format == *format && a.path == relative)
                .ok_or_else(|| {
                    AppError::new(
                        "EXPORT_DESTINATION_EXISTS_OR_DENIED",
                        "目标文件已存在且不属于此次任务",
                    )
                })?;
            if !matches_artifact(&path, old)? {
                return Err(AppError::new(
                    "EXPORT_ARTIFACT_CHANGED",
                    "产物被修改，保留文件且不会覆盖；可新建一次运行",
                ));
            }
            let mut artifact = old.clone();
            artifact.published = true;
            app.record_export_artifact_local(account, id, artifact)?;
            continue;
        }
        let mut artifact = export_file::stream_new(&path, |file| {
            let mut out = HashWriter {
                file,
                hash: Sha256::new(),
                size: 0,
            };
            let summary =
                app.export_capture_local(account, &run.capture_id, format, &options, id, &mut out)?;
            let artifact = ExportArtifact {
                format: format.clone(),
                path: relative.clone(),
                sha256: hex::encode(out.hash.finalize()),
                size: out.size,
                rows: summary.rows,
                published: false,
            };
            // Journal exact expected bytes before publishing. A crash between these steps is recoverable.
            app.record_export_artifact_local(account, id, artifact.clone())?;
            Ok(artifact)
        })?;
        artifact.published = true;
        app.record_export_artifact_local(account, id, artifact)?;
    }
    Ok(())
}
pub(super) fn recover(app: &Application) -> AppResult<()> {
    for account in app.accounts_local()? {
        let mut before = None;
        loop {
            let (items, next) = app.export_runs_local(&account.id, before)?;
            for item in items {
                if matches!(item.run.state.as_str(), "queued" | "reading" | "exporting") {
                    app.finish_export_run_local(
                        &account.id,
                        &item.id,
                        "interrupted",
                        Some(AppError::new(
                            "APP_RESTARTED",
                            "上次运行被中断，可继续已保存的进度",
                        )),
                    )?;
                }
            }
            before = next;
            if before.is_none() {
                break;
            }
        }
    }
    Ok(())
}
fn tick(app: &Application, downloads: &Path, allow_live: bool) -> AppResult<bool> {
    for account in app.accounts_local()? {
        let mut before = None;
        loop {
            let (items, next) = app.export_plans_local(&account.id, before)?;
            for item in items {
                let run = if let Some(id) = item.plan.active_run {
                    app.export_run_local(&account.id, &id)?
                } else {
                    match app.begin_export_run_local(&account.id, &item.id, false) {
                        Ok(run) => run,
                        Err(e)
                            if matches!(
                                e.code.as_str(),
                                "NOT_DUE" | "BUSY" | "DOCUMENT_CONFLICT"
                            ) =>
                        {
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                };
                let id = run.id.clone();
                let result = execute(app, &account.id, run, downloads, allow_live);
                {
                    match result {
                        Ok(()) => {
                            app.finish_export_run_local(&account.id, &id, "succeeded", None)?;
                        }
                        Err(e) => {
                            let state = if e.code == "QUERY_CANCELED" {
                                "canceled"
                            } else {
                                "failed"
                            };
                            app.finish_export_run_local(&account.id, &id, state, Some(e))?;
                        }
                    }
                }
                return Ok(true);
            }
            before = next;
            if before.is_none() {
                break;
            }
        }
    }
    Ok(false)
}
pub(super) fn start(
    app: Arc<Application>,
    downloads: PathBuf,
    allow_live: bool,
    handle: tauri::AppHandle,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            let worker = app.clone();
            let output = downloads.clone();
            match tauri::async_runtime::spawn_blocking(move || tick(&worker, &output, allow_live))
                .await
            {
                Ok(Ok(true)) => notifications::task_finished_at(&handle, "library"),
                Ok(Err(e)) => diagnostics::record(&handle, "task", &e),
                _ => {}
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}
