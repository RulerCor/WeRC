use super::*;
use werc_application::directory_export::{
    DirectoryExportSummary, DirectoryFormat, DirectorySelection,
};

#[tauri::command]
pub(super) async fn save_directory_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    selection: DirectorySelection,
    format: DirectoryFormat,
    path: String,
    request_id: String,
) -> Result<DirectoryExportSummary, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
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
        app.export_live_directory_local(
            &account_id,
            &view_id,
            &selection,
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
pub(super) async fn save_live_message_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    selection: werc_application::live_export::LiveExportSelection,
    format: werc_application::live_export::LiveExportFormat,
    path: String,
    request_id: String,
) -> Result<werc_application::live_export::LiveExportSummary, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
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
        app.export_live_messages_local(
            &account_id,
            &view_id,
            &selection,
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
pub(super) async fn show_directory_export(
    handle: tauri::AppHandle,
    path: String,
) -> Result<(), AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    let destination = export_file::destination(&path, &downloads)?;
    if !destination.is_file() {
        return Err(AppError::new("EXPORT_NOT_FOUND", "导出文件已经移动或删除"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let windows = std::env::var_os("WINDIR")
            .ok_or_else(|| AppError::new("EXPORT_REVEAL_FAILED", "无法定位系统文件管理器"))?;
        std::process::Command::new(std::path::Path::new(&windows).join("explorer.exe"))
            .arg("/select,")
            .arg(destination)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| AppError::new("EXPORT_REVEAL_FAILED", "无法打开文件所在目录"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(AppError::new(
            "UNSUPPORTED_PLATFORM",
            "当前平台未提供文件定位",
        ))
    }
}
