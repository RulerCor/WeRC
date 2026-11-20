use super::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use werc_domain::AppResult;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Preferences {
    diagnostics_logging: bool,
    message_notifications: bool,
    notification_preview: bool,
    task_notifications: bool,
    close_behavior: String,
    launch_on_login: bool,
    model_directory: String,
    asr_provider:String,
    asr_threads:u32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            diagnostics_logging: true,
            message_notifications: true,
            notification_preview: false,
            task_notifications: true,
            close_behavior: "tray".into(),
            launch_on_login: false,
            model_directory: "models".into(),
            asr_provider:"cpu".into(),
            asr_threads:2,
        }
    }
}
pub(super) struct PreferencesStore {
    directory: PathBuf,
    value: Mutex<Preferences>,
}
impl PreferencesStore {
    pub fn open(directory: PathBuf) -> AppResult<Self> {
        let value = match std::fs::read(directory.join("preferences.json")) {
            Ok(bytes) => serde_json::from_slice::<Preferences>(&bytes).map_err(|_| {
                AppError::new(
                    "SETTINGS_INVALID",
                    "本机设置无法读取，请检查preferences.json",
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Preferences::default(),
            Err(_) => return Err(AppError::new("SETTINGS_IO_FAILED", "无法读取本机设置")),
        };
        werc_wechat_win::asr::configure_models_execution_with(resolve_models(
            &directory,
            &value.model_directory,
        )?,&value.asr_provider,value.asr_threads,||Ok(()))?;
        Ok(Self {
            directory,
            value: Mutex::new(value),
        })
    }
    pub fn notifications(&self) -> (bool, bool, bool) {
        self.value
            .lock()
            .map(|v| {
                (
                    v.message_notifications,
                    v.notification_preview,
                    v.task_notifications,
                )
            })
            .unwrap_or((false, false, false))
    }
    pub fn diagnostics_logging(&self) -> bool {
        self.value
            .lock()
            .map(|v| v.diagnostics_logging)
            .unwrap_or(false)
    }
    pub fn close_behavior(&self) -> String {
        self.value
            .lock()
            .map(|v| v.close_behavior.clone())
            .unwrap_or_else(|_| "tray".into())
    }
}
fn resolve_models(base: &std::path::Path, value: &str) -> AppResult<PathBuf> {
    let path = PathBuf::from(value);
    if value.trim().is_empty()
        || value.chars().any(char::is_control)
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(AppError::new("INVALID_ARGUMENT", "模型目录无效"));
    }
    Ok(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}
#[tauri::command]
pub(super) fn desktop_preferences(state: State<'_, PreferencesStore>) -> AppResult<Preferences> {
    let mut value = state
        .value
        .lock()
        .map_err(|_| AppError::internal())?
        .clone();
    value.launch_on_login = autostart_enabled()?;
    Ok(value)
}
#[tauri::command]
pub(super) fn save_desktop_preferences(
    handle: tauri::AppHandle,
    state: State<'_, PreferencesStore>,
    value: Preferences,
    models: State<'_, models::Models>,
) -> AppResult<Preferences> {
    if !matches!(value.close_behavior.as_str(), "ask" | "tray" | "exit") {
        return Err(AppError::new("INVALID_ARGUMENT", "关闭行为无效"));
    }
    if handle.config().identifier != "local.werc.mon3trbot" && value.launch_on_login {
        return Err(AppError::new("FORBIDDEN", "隔离验证配置不注册开机启动"));
    }
    models.while_idle(|| {
        let directory = resolve_models(&state.directory, &value.model_directory)?;
        if value.asr_provider=="cuda" { gpu::require_ready(&directory)?; }
        std::fs::create_dir_all(&directory)
            .map_err(|_| AppError::new("SETTINGS_IO_FAILED", "无法创建模型目录"))?;
        werc_wechat_win::asr::configure_models_execution_with(directory, &value.asr_provider.clone(), value.asr_threads, || {
            let mut guard = state.value.lock().map_err(|_| AppError::internal())?;
            let previous = autostart_enabled()?;
            if previous != value.launch_on_login {
                set_autostart(value.launch_on_login)?;
            }
            if let Err(error) = save(&state.directory, &value) {
                if previous != value.launch_on_login {
                    let _ = set_autostart(previous);
                }
                return Err(error);
            }
            *guard = value.clone();
            Ok(value)
        })
    })
}
fn save(directory: &std::path::Path, value: &Preferences) -> AppResult<()> {
    use std::io::Write;
    let temp = directory.join(format!("preferences-{}.tmp", uuid::Uuid::new_v4()));
    let destination = directory.join("preferences.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| AppError::new("SETTINGS_IO_FAILED", "无法保存本机设置"))?;
    let result = (|| {
        file.write_all(&serde_json::to_vec_pretty(value).map_err(|_| AppError::internal())?)
            .and_then(|_| file.sync_all())
            .map_err(|_| AppError::new("SETTINGS_IO_FAILED", "设置写盘失败"))
    })();
    drop(file);
    let result = result.and_then(|_| replace(&temp, &destination));
    let _ = std::fs::remove_file(temp);
    result
}
pub(super) fn replace(source: &std::path::Path, destination: &std::path::Path) -> AppResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::*;
        let a: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            MoveFileExW(
                a.as_ptr(),
                b.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(AppError::new("SETTINGS_IO_FAILED", "无法更新本机设置"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(source, destination)
            .map_err(|_| AppError::new("SETTINGS_IO_FAILED", "无法更新本机设置"))
    }
}
#[cfg(windows)]
fn autostart_key() -> Vec<u16> {
    "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16()
        .collect()
}
#[cfg(windows)]
fn autostart_enabled() -> AppResult<bool> {
    use windows_sys::Win32::System::Registry::*;
    let key = autostart_key();
    let name: Vec<_> = "WeRC_Mon3trBot\0".encode_utf16().collect();
    let mut bytes = 0;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if code == 2 {
        return Ok(false);
    }
    if code != 0 {
        return Err(AppError::new("SETTINGS_IO_FAILED", "无法读取开机启动状态"));
    }
    Ok(bytes > 2)
}
#[cfg(windows)]
fn set_autostart(enabled: bool) -> AppResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::*;
    let key = autostart_key();
    let name: Vec<_> = "WeRC_Mon3trBot\0".encode_utf16().collect();
    let mut handle = std::ptr::null_mut();
    if unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut handle,
            std::ptr::null_mut(),
        )
    } != 0
    {
        return Err(AppError::new("SETTINGS_IO_FAILED", "无法修改开机启动设置"));
    }
    let result = if enabled {
        let exe = std::env::current_exe().map_err(|_| AppError::internal());
        match exe {
            Ok(exe) => {
                let mut value = vec![b'"' as u16];
                value.extend(exe.as_os_str().encode_wide());
                value.extend([b'"' as u16, 0]);
                unsafe {
                    RegSetValueExW(
                        handle,
                        name.as_ptr(),
                        0,
                        REG_SZ,
                        value.as_ptr().cast(),
                        (value.len() * 2) as u32,
                    )
                }
            }
            Err(_) => 1,
        }
    } else {
        unsafe { RegDeleteValueW(handle, name.as_ptr()) }
    };
    unsafe { RegCloseKey(handle) };
    if result == 0 || (!enabled && result == 2) {
        Ok(())
    } else {
        Err(AppError::new("SETTINGS_IO_FAILED", "开机启动设置未保存"))
    }
}
#[cfg(not(windows))]
fn autostart_enabled() -> AppResult<bool> {
    Ok(false)
}
#[cfg(not(windows))]
fn set_autostart(_: bool) -> AppResult<()> {
    Err(AppError::new(
        "UNSUPPORTED_PLATFORM",
        "仅支持Windows开机启动",
    ))
}

#[tauri::command]
pub(super) fn close_desktop(
    handle: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, DesktopState>,
    action: String,
) -> AppResult<()> {
    match action.as_str() {
        "tray" => window.hide().map_err(|_| AppError::internal()),
        "exit" => {
            state.application.enable_api(false);
            state.native_provider.disconnect(None)?;
            handle.exit(0);
            Ok(())
        }
        _ => Err(AppError::new("INVALID_ARGUMENT", "关闭操作无效")),
    }
}
pub(super) fn on_close(window: &tauri::Window) {
    use tauri::Emitter;
    match window.state::<PreferencesStore>().close_behavior().as_str() {
        "exit" => {
            let state = window.state::<DesktopState>();
            state.application.enable_api(false);
            let _ = state.native_provider.disconnect(None);
            window.app_handle().exit(0);
        }
        "ask" => {
            let _ = window.show();
            let _ = window.set_focus();
            let _ = window.emit("werc-close-request", ());
        }
        _ => {
            let _ = window.hide();
        }
    }
}
