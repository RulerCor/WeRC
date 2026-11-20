use super::*;
use werc_domain::special_records::RecordQuery;
fn require_live(handle: &tauri::AppHandle) -> Result<(), AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不读取真实专项记录"));
    }
    Ok(())
}
#[tauri::command]
pub(super) async fn native_special_records(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    query: Option<RecordQuery>,
    cursor: Option<String>,
    request_id: String,
) -> Result<werc_application::live_special_records::SpecialRecordPage, AppError> {
    require_live(&handle)?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.live_special_records_local(&account_id, &view_id, query, cursor.as_deref(), &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[derive(serde::Serialize)]
pub(super) struct Summary {
    rows: usize,
    bytes: usize,
}
#[tauri::command]
pub(super) async fn save_special_records(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    query: RecordQuery,
    request_id: String,
    format: String,
    path: String,
) -> Result<Summary, AppError> {
    require_live(&handle)?;
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|s| s.to_str()) != Some(format.as_str()) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "扩展名必须与导出格式一致",
            ));
        }
        if destination.exists() {
            return Err(AppError::new(
                "EXPORT_DESTINATION_EXISTS_OR_DENIED",
                "目标文件已存在，请换一个文件名",
            ));
        }
        app.export_special_records_local(
            &account_id,
            &view_id,
            query,
            &request_id,
            &format,
            |bytes, rows| {
                export_file::write_new(&destination, bytes)?;
                Ok(Summary {
                    rows,
                    bytes: bytes.len(),
                })
            },
        )
    })
    .await
    .map_err(|_| AppError::internal())?
}
