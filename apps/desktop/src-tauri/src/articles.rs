use super::*;
use werc_application::articles::{ArticleAction, ArticleExportFormat};
#[tauri::command]
pub(super) async fn article_job_action(
    state: State<'_, DesktopState>,
    account_id: String,
    operation: werc_application::article_jobs::JobAction,
) -> Result<serde_json::Value, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.article_job_action_local(&account_id, operation)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn article_action(
    state: State<'_, DesktopState>,
    account_id: String,
    operation: ArticleAction,
    request_id: String,
) -> Result<serde_json::Value, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.article_action_local(&account_id, operation, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn article_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    ids: Vec<String>,
    format: ArticleExportFormat,
    path: String,
    request_id: String,
) -> Result<serde_json::Value, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move||{
        let dest=export_file::destination(&path,&downloads)?;
        if dest.extension().and_then(|s|s.to_str())!=Some(format.extension()){return Err(AppError::new("INVALID_ARGUMENT","文件名与文章导出格式不符"));}
        let output=app.export_articles_local(&account_id,&ids,format,&request_id)?;
        export_file::stream_new(&dest,|file|{use std::io::Write;file.write_all(&output.bytes).map_err(|_|AppError::new("EXPORT_WRITE_FAILED","文章导出写入失败"))?;Ok(())})?;
        Ok(serde_json::json!({"rows":output.rows,"external_images":output.external_images,"bytes":output.bytes.len(),"path":path}))
    }).await.map_err(|_|AppError::internal())?
}
