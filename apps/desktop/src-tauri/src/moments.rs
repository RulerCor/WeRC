use super::*;
use werc_application::live_moments::{MomentExportFormat, MomentExportSummary, MomentOverview};
use werc_domain::{live::LiveCalendarDay, moments::MomentQuery};
fn require_live(handle: &tauri::AppHandle) -> Result<(), AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实朋友圈"));
    }
    Ok(())
}
#[tauri::command]
pub(super) async fn native_moment_overview(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    query: MomentQuery,
    days: Vec<LiveCalendarDay>,
    request_id: String,
) -> Result<MomentOverview, AppError> {
    require_live(&handle)?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.moment_overview_local(&account_id, &view_id, &query, &days, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn save_moment_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    query: MomentQuery,
    format: MomentExportFormat,
    path: String,
    request_id: String,
) -> Result<MomentExportSummary, AppError> {
    require_live(&handle)?;
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|s| s.to_str()) != Some(format.extension()) {
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
        app.export_live_moments_local(
            &account_id,
            &view_id,
            &query,
            format,
            &request_id,
            |output| {
                export_file::write_new(&destination, &output.bytes)?;
                Ok(output.summary)
            },
        )
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn moment_preservation_status(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<werc_application::moment_preservation::PreservationStatus, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.moment_preservation_status_local(&account_id))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn configure_moment_preservation(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    enabled: bool,
) -> Result<werc_application::moment_preservation::PreservationStatus, AppError> {
    require_live(&handle)?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.configure_moment_preservation_local(&account_id, enabled)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn preserved_moments(
    state: State<'_, DesktopState>,
    account_id: String,
    before: Option<i64>,
    query: MomentQuery,
) -> Result<werc_application::moment_preservation::PreservedMomentPage, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.preserved_moments_local(&account_id, before, &query)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn save_preserved_moments(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    query: MomentQuery,
    format: MomentExportFormat,
    path: String,
    request_id: String,
) -> Result<MomentExportSummary, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        if destination.extension().and_then(|v| v.to_str()) != Some(format.extension()) {
            return Err(AppError::new("INVALID_ARGUMENT", "扩展名与导出格式不符"));
        }
        let output =
            app.export_preserved_moments_local(&account_id, &query, format, &request_id)?;
        export_file::write_new(&destination, &output.bytes)?;
        Ok(output.summary)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn native_moment_media(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: Option<String>,
    selection: werc_domain::moments::MomentMediaSelection,
    request_id: String,
) -> Result<werc_application::moment_media::MomentMediaManifest, AppError> {
    require_live(&handle)?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(media) = app.cached_moment_media_local(&account_id, &selection)? {
            return Ok(media);
        }
        let view = view_id.ok_or_else(|| {
            AppError::new(
                "MOMENT_MEDIA_NOT_SAVED",
                "此媒体尚未保留，请在在线朋友圈中载入一次",
            )
        })?;
        app.fetch_moment_media_local(&account_id, &view, &selection, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[derive(Serialize)]
pub(super) struct MomentPreview {
    mime: String,
    bytes: Vec<u8>,
}
#[tauri::command]
pub(super) async fn moment_media_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<MomentPreview, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (m, b) = app.moment_media_contents_local(&account_id, &id)?;
        Ok(MomentPreview {
            mime: m.mime,
            bytes: b.to_vec(),
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn save_moment_media(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        let (_, bytes) = app.moment_media_contents_local(&account_id, &id)?;
        export_file::write_new(&destination, &bytes)?;
        Ok(bytes.len())
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn save_moment_bundle(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: Option<String>,
    query: MomentQuery,
    request_id: String,
    path: String,
) -> Result<werc_application::moment_bundle::MomentBundleSummary, AppError> {
    if view_id.is_some() {
        require_live(&handle)?;
    }
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::internal())?;
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let destination = export_file::destination(&path, &downloads)?;
        export_file::stream_new(&destination, |out| {
            app.export_moment_bundle_local(
                &account_id,
                view_id.as_deref(),
                &query,
                &request_id,
                out,
            )
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}
