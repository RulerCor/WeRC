use super::*;
fn require_live(handle: &tauri::AppHandle) -> Result<(), AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实收藏"));
    }
    Ok(())
}
#[tauri::command]
pub(super) async fn native_live_favorites(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    query: Option<werc_domain::favorites::FavoriteQuery>,
    cursor: Option<String>,
    request_id: String,
) -> Result<werc_application::live_favorites::FavoritePage, AppError> {
    require_live(&handle)?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.live_favorites_local(&account_id, &view_id, query, cursor.as_deref(), &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_favorite_copy(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    item_token: String,
    request_id: String,
) -> Result<werc_application::library::LibraryItem, AppError> {
    require_live(&handle)?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.copy_favorite_local(&account_id, &view_id, &item_token, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[derive(serde::Serialize)]
pub(super) struct Preview {
    mime: String,
    bytes: Vec<u8>,
}
#[tauri::command]
pub(super) async fn native_favorite_preview(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    item_token: String,
    slot: Vec<usize>,
    request_id: String,
) -> Result<Preview, AppError> {
    require_live(&handle)?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let output =
            app.favorite_media_local(&account_id, &view_id, &item_token, &slot, &request_id)?;
        if output.mime == "application/octet-stream" {
            return Err(AppError::new("INVALID_ARGUMENT", "附件请保存后打开"));
        }
        Ok(Preview {
            mime: output.mime.into(),
            bytes: output.file.bytes.to_vec(),
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_favorite_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    item_token: String,
    slot: Vec<usize>,
    request_id: String,
    path: String,
) -> Result<usize, AppError> {
    require_live(&handle)?;
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        let output =
            app.favorite_media_local(&account_id, &view_id, &item_token, &slot, &request_id)?;
        export_file::stream_new(&destination, |file| {
            use std::io::Write;
            file.write_all(&output.file.bytes)
                .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "保存收藏媒体失败"))?;
            Ok(output.file.bytes.len())
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}
