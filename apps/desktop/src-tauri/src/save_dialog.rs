use super::*;
use werc_application::live_export::LiveExportFormat;

#[tauri::command]
pub(super) async fn choose_local_file(
    window: tauri::WebviewWindow,
    kind: String,
) -> Result<Option<String>, AppError> {
    let (title, filter) = match kind.as_str() {
        "model" => ("选择本地GGML模型", "GGML模型\0*.bin\0\0"),
        "backup" => ("选择WeRC资料备份", "WeRC备份\0*.werc\0\0"),
        "image_sample" => ("选择微信加密图片样本", "微信图片缓存\0*.dat\0所有文件\0*.*\0\0"),
        _ => return Err(AppError::new("INVALID_ARGUMENT", "文件类型无效")),
    };
    #[cfg(windows)]
    {
        let owner = window.hwnd().map_err(|_| AppError::internal())?.0 as usize;
        tauri::async_runtime::spawn_blocking(move || {
            use windows_sys::Win32::UI::Controls::Dialogs::*;
            let mut buffer = vec![0u16; 32768];
            let title: Vec<_> = title.encode_utf16().chain(Some(0)).collect();
            let filter: Vec<_> = filter.encode_utf16().collect();
            let mut dialog = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: owner as _,
                lpstrFilter: filter.as_ptr(),
                nFilterIndex: 1,
                lpstrFile: buffer.as_mut_ptr(),
                nMaxFile: buffer.len() as u32,
                lpstrTitle: title.as_ptr(),
                Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
                ..Default::default()
            };
            if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
                return if unsafe { CommDlgExtendedError() } == 0 {
                    Ok(None)
                } else {
                    Err(AppError::new(
                        "FILE_DIALOG_FAILED",
                        "无法打开系统文件选择器",
                    ))
                };
            }
            let end = buffer
                .iter()
                .position(|n| *n == 0)
                .ok_or_else(AppError::internal)?;
            Ok(Some(
                String::from_utf16(&buffer[..end]).map_err(|_| AppError::internal())?,
            ))
        })
        .await
        .map_err(|_| AppError::internal())?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, title, filter);
        Err(AppError::new(
            "UNSUPPORTED_PLATFORM",
            "当前平台不支持文件选择器",
        ))
    }
}

#[tauri::command]
pub(super) async fn choose_directory_export_path(
    handle: tauri::AppHandle,
    window: tauri::WebviewWindow,
    format: LiveExportFormat,
    suggested_name: String,
) -> Result<Option<String>, AppError> {
    choose_export_path(handle, window, format.extension().into(), suggested_name, false).await
}
#[tauri::command]
pub(super) async fn choose_media_save_path(
    handle: tauri::AppHandle,
    window: tauri::WebviewWindow,
    suggested_name: String,
    allow_existing: Option<bool>,
) -> Result<Option<String>, AppError> {
    let extension = std::path::Path::new(&suggested_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("bin")
        .to_owned();
    if extension.is_empty()
        || extension.len() > 16
        || !extension.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(AppError::new("INVALID_ARGUMENT", "媒体文件扩展名无效"));
    }
    choose_export_path(handle, window, extension, suggested_name, allow_existing.unwrap_or(false)).await
}
async fn choose_export_path(
    handle: tauri::AppHandle,
    window: tauri::WebviewWindow,
    file_extension: String,
    suggested_name: String,
    allow_existing: bool,
) -> Result<Option<String>, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    #[cfg(windows)]
    {
        let owner = window.hwnd().map_err(|_| AppError::internal())?.0 as usize;
        tauri::async_runtime::spawn_blocking(move || {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::UI::Controls::Dialogs::*;
            let name = std::path::Path::new(&suggested_name)
                .file_name()
                .and_then(|s| s.to_str())
                .filter(|s| !s.contains('\0') && s.len() < 512)
                .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "建议文件名无效"))?;
            let mut buffer = vec![0u16; 32768];
            let seed: Vec<_> = name.encode_utf16().collect();
            buffer[..seed.len()].copy_from_slice(&seed);
            let initial: Vec<_> = downloads
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let filter: Vec<_> = format!(
                "{} 文件\0*.{}\0\0",
                file_extension.to_uppercase(),
                file_extension
            )
            .encode_utf16()
            .collect();
            let extension: Vec<_> = file_extension
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let title: Vec<_> = "选择导出文件保存位置\0".encode_utf16().collect();
            let mut dialog = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: owner as _,
                lpstrFilter: filter.as_ptr(),
                nFilterIndex: 1,
                lpstrFile: buffer.as_mut_ptr(),
                nMaxFile: buffer.len() as u32,
                lpstrInitialDir: initial.as_ptr(),
                lpstrTitle: title.as_ptr(),
                lpstrDefExt: extension.as_ptr(),
                Flags: OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY,
                ..Default::default()
            };
            // Every UTF-16 buffer remains alive for the modal call.
            if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
                return if unsafe { CommDlgExtendedError() } == 0 {
                    Ok(None)
                } else {
                    Err(AppError::new(
                        "EXPORT_DIALOG_FAILED",
                        "系统保存对话框无法完成，可直接填写下载目录内文件名",
                    ))
                };
            }
            let end = buffer
                .iter()
                .position(|n| *n == 0)
                .ok_or_else(AppError::internal)?;
            let selected = String::from_utf16(&buffer[..end])
                .map_err(|_| AppError::new("INVALID_ARGUMENT", "保存路径编码无效"))?;
            let destination = export_file::destination(&selected, &downloads)?;
            if !allow_existing && destination.exists() {
                return Err(AppError::new(
                    "EXPORT_DESTINATION_EXISTS_OR_DENIED",
                    "该文件已存在，请选择其他文件名",
                ));
            }
            Ok(Some(selected))
        })
        .await
        .map_err(|_| AppError::internal())?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, file_extension, suggested_name, downloads, allow_existing);
        Err(AppError::new(
            "UNSUPPORTED_PLATFORM",
            "当前平台未提供保存对话框，请填写文件名",
        ))
    }
}

#[tauri::command]
pub(super) async fn choose_storage_directory(
    window: tauri::WebviewWindow,
) -> Result<Option<String>, AppError> {
    #[cfg(windows)]
    {
        let owner = window.hwnd().map_err(|_| AppError::internal())?.0 as usize;
        tauri::async_runtime::spawn_blocking(move || {
            use windows::Win32::{Foundation::HWND, System::Com::*, UI::Shell::*};
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                    .ok()
                    .map_err(|_| AppError::new("FILE_DIALOG_FAILED", "无法初始化目录选择器"))?;
                struct Apartment;
                impl Drop for Apartment {
                    fn drop(&mut self) {
                        unsafe { CoUninitialize() }
                    }
                }
                let _apartment = Apartment;
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                        .map_err(|_| AppError::internal())?;
                dialog
                    .SetOptions(
                        FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_NOCHANGEDIR,
                    )
                    .map_err(|_| AppError::internal())?;
                dialog
                    .SetTitle(windows::core::w!("选择 WeRC 资料保存位置"))
                    .map_err(|_| AppError::internal())?;
                if let Err(e) = dialog.Show(Some(HWND(owner as _))) {
                    if e.code().0 as u32 == 0x800704c7 {
                        return Ok(None);
                    }
                    return Err(AppError::new("FILE_DIALOG_FAILED", "目录选择器未能完成"));
                }
                let item = dialog.GetResult().map_err(|_| AppError::internal())?;
                let name = item
                    .GetDisplayName(SIGDN_FILESYSPATH)
                    .map_err(|_| AppError::internal())?;
                let result = name.to_string().map_err(|_| AppError::internal());
                CoTaskMemFree(Some(name.0.cast()));
                Ok(Some(result?))
            }
        })
        .await
        .map_err(|_| AppError::internal())?
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err(AppError::new(
            "UNSUPPORTED_PLATFORM",
            "此目录选择器仅用于Windows",
        ))
    }
}
