//! Signed updates use build-owned endpoints and keys, never a Bot-supplied URL.
use super::*;
use std::{path::PathBuf, time::Duration};
use tauri_plugin_updater::{Update, UpdaterExt};
use werc_application::ports::Repository;
use werc_domain::AppResult;
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Serialize)]
pub(super) struct UpdateStatus {
    configured: bool,
    current_version: String,
    phase: String,
    version: Option<String>,
    notes: Option<String>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    release_page_available: bool,
    error: Option<AppError>,
}
impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            configured: configuration().is_ok(),
            current_version: env!("CARGO_PKG_VERSION").into(),
            phase: "idle".into(),
            version: None,
            notes: None,
            downloaded_bytes: 0,
            total_bytes: None,
            release_page_available: release_page().is_ok(),
            error: None,
        }
    }
}
#[derive(Default)]
struct Inner {
    status: UpdateStatus,
    available: Option<Update>,
    package: Option<Package>,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
}
struct Package {
    path: PathBuf,
}
impl Drop for Package {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
#[derive(Clone, Default)]
pub(super) struct Updates(Arc<Mutex<Inner>>);
fn error(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}
fn https(value: &str) -> AppResult<reqwest::Url> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| error("UPDATE_NOT_CONFIGURED", "此版本尚未配置有效的更新地址"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(error(
            "UPDATE_NOT_CONFIGURED",
            "此版本尚未配置有效的更新地址",
        ));
    }
    Ok(url)
}
fn configuration() -> AppResult<(reqwest::Url, &'static str)> {
    let endpoint = option_env!("WERC_UPDATE_ENDPOINT").unwrap_or("");
    let key = option_env!("WERC_UPDATE_PUBLIC_KEY").unwrap_or("").trim();
    if key.is_empty() {
        return Err(error(
            "UPDATE_NOT_CONFIGURED",
            "此版本通过安装包手动更新，可继续使用当前版本",
        ));
    }
    Ok((https(endpoint)?, key))
}
fn release_page() -> AppResult<reqwest::Url> {
    https(option_env!("WERC_RELEASE_PAGE").unwrap_or(""))
}
fn busy(status: &UpdateStatus) -> bool {
    matches!(
        status.phase.as_str(),
        "checking" | "downloading" | "installing"
    )
}
impl Updates {
    pub(super) fn clear_cache(&self, directory: &std::path::Path) -> AppResult<usize> {
        let mut inner = self.0.lock().map_err(|_| AppError::internal())?;
        if busy(&inner.status) {
            return Err(error("BUSY", "更新操作正在进行，请稍后清理"));
        }
        inner.package = None;
        if inner.status.phase == "ready" {
            inner.status.phase = "available".into();
        }
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(_) => return Err(error("UPDATE_IO_FAILED", "无法读取更新缓存目录")),
        };
        let mut count = 0;
        for entry in entries {
            let entry = entry.map_err(|_| error("UPDATE_IO_FAILED", "无法读取更新缓存"))?;
            let path = entry.path();
            if path.extension().is_none_or(|v| v != "package")
                || path
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .is_none_or(|v| uuid::Uuid::parse_str(v).is_err())
            {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|_| error("UPDATE_IO_FAILED", "无法读取更新缓存"))?;
            if !meta.is_file() || !super::storage::ordinary(&meta) {
                continue;
            }
            std::fs::remove_file(&path)
                .map_err(|_| error("UPDATE_IO_FAILED", "更新缓存清理失败"))?;
            count += 1;
        }
        Ok(count)
    }
    fn status(&self) -> AppResult<UpdateStatus> {
        Ok(self
            .0
            .lock()
            .map_err(|_| AppError::internal())?
            .status
            .clone())
    }
    fn failed(&self, e: AppError) -> AppResult<UpdateStatus> {
        let mut inner = self.0.lock().map_err(|_| AppError::internal())?;
        inner.status.phase = if e.code == "QUERY_CANCELED" {
            "canceled"
        } else {
            "failed"
        }
        .into();
        inner.status.error = Some(e);
        inner.cancel = None;
        Ok(inner.status.clone())
    }
}
#[tauri::command]
pub(super) fn update_status(state: State<'_, Updates>) -> AppResult<UpdateStatus> {
    state.status()
}
#[tauri::command]
pub(super) async fn check_for_update(
    app: tauri::AppHandle,
    state: State<'_, Updates>,
) -> AppResult<UpdateStatus> {
    let (endpoint, key) = match configuration() {
        Ok(c) => c,
        Err(e) => return state.failed(e),
    };
    {
        let mut inner = state.0.lock().map_err(|_| AppError::internal())?;
        if busy(&inner.status) {
            return Err(error("BUSY", "更新操作正在进行"));
        }
        inner.status = UpdateStatus {
            phase: "checking".into(),
            ..Default::default()
        };
        inner.available = None;
        inner.package = None;
    }
    let result = async {
        let updater = app
            .updater_builder()
            .pubkey(key)
            .endpoints(vec![endpoint])
            .map_err(|_| error("UPDATE_NOT_CONFIGURED", "更新地址无效"))?
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| error("UPDATE_NOT_CONFIGURED", "更新组件配置无效"))?;
        let update = updater.check().await.map_err(|_| {
            error(
                "UPDATE_CHECK_FAILED",
                "检查更新失败，请稍后重试或查看发布页",
            )
        })?;
        if update
            .as_ref()
            .is_some_and(|u| https(u.download_url.as_str()).is_err())
        {
            return Err(error(
                "UPDATE_SOURCE_INVALID",
                "更新文件没有使用有效的HTTPS地址",
            ));
        }
        Ok(update)
    }
    .await;
    match result {
        Ok(update) => {
            let mut inner = state.0.lock().map_err(|_| AppError::internal())?;
            if let Some(u) = &update {
                inner.status.version = Some(u.version.clone());
                inner.status.notes = u.body.as_ref().map(|s| s.chars().take(8000).collect());
                inner.status.phase = "available".into();
            } else {
                inner.status.phase = "up_to_date".into();
            }
            inner.available = update;
            Ok(inner.status.clone())
        }
        Err(e) => state.failed(e),
    }
}
#[tauri::command]
pub(super) async fn download_update(
    app: tauri::AppHandle,
    state: State<'_, Updates>,
) -> AppResult<UpdateStatus> {
    let directory = app
        .path()
        .app_cache_dir()
        .map_err(|_| error("UPDATE_IO_FAILED", "无法定位更新缓存目录"))?
        .join("updates");
    let (update, mut canceled) = {
        let mut inner = state.0.lock().map_err(|_| AppError::internal())?;
        if busy(&inner.status) {
            return Err(error("BUSY", "更新操作正在进行"));
        }
        let update = inner
            .available
            .clone()
            .ok_or_else(|| error("UPDATE_NOT_AVAILABLE", "请先检查更新"))?;
        let (cancel, receiver) = tokio::sync::watch::channel(false);
        inner.cancel = Some(cancel);
        inner.package = None;
        inner.status.phase = "downloading".into();
        inner.status.error = None;
        inner.status.downloaded_bytes = 0;
        inner.status.total_bytes = None;
        (update, receiver)
    };
    let task = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let progress = task.clone();
        let result=async {
            let download=update.download(move |length,total|{
                if let Ok(mut inner)=progress.0.lock(){
                    inner.status.downloaded_bytes=inner.status.downloaded_bytes.saturating_add(length as u64);
                    inner.status.total_bytes=total;
                    if inner.status.downloaded_bytes>MAX_PACKAGE_BYTES || total.is_some_and(|n|n>MAX_PACKAGE_BYTES){
                        inner.status.error=Some(error("UPDATE_TOO_LARGE","更新文件超过512 MiB上限，已停止下载"));
                        if let Some(cancel)=&inner.cancel{let _=cancel.send(true);}
                    }
                }
            },||{});
            let bytes=tokio::select! {
                _=canceled.changed()=>return Err(error("QUERY_CANCELED","更新下载已取消")),
                result=download=>result.map_err(|_|error("UPDATE_DOWNLOAD_FAILED","更新下载或签名校验失败，请重新检查更新"))?,
            };
            if *canceled.borrow(){return Err(error("QUERY_CANCELED","更新下载已取消"));}
            if bytes.len() as u64>MAX_PACKAGE_BYTES{return Err(error("UPDATE_TOO_LARGE","更新文件超过512 MiB上限"));}
            std::fs::create_dir_all(&directory).map_err(|_|error("UPDATE_IO_FAILED","无法创建更新缓存，请检查可用空间"))?;
            let package=Package{path:directory.join(format!("{}.package",uuid::Uuid::new_v4()))};
            use std::io::Write;
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&package.path)
                .map_err(|_|error("UPDATE_IO_FAILED","无法创建更新缓存文件"))?;
            file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|error("UPDATE_IO_FAILED","更新文件写入失败，请检查可用空间"))?;
            drop(file);
            let mut inner=task.0.lock().map_err(|_|AppError::internal())?;
            if *canceled.borrow(){return Err(error("QUERY_CANCELED","更新下载已取消"));}
            inner.package=Some(package);inner.cancel=None;inner.status.phase="ready".into();Ok(())
        }.await;
        if let Err(e) = result {
            let failure = task
                .0
                .lock()
                .ok()
                .and_then(|i| i.status.error.clone())
                .unwrap_or(e);
            let _ = task.failed(failure);
        }
    });
    state.status()
}
#[tauri::command]
pub(super) fn cancel_update_download(state: State<'_, Updates>) -> AppResult<UpdateStatus> {
    let inner = state.0.lock().map_err(|_| AppError::internal())?;
    if let Some(cancel) = &inner.cancel {
        let _ = cancel.send(true);
    }
    drop(inner);
    state.status()
}
#[tauri::command]
pub(super) async fn install_update(
    app: tauri::AppHandle,
    state: State<'_, Updates>,
) -> AppResult<UpdateStatus> {
    let (update, package) = {
        let mut inner = state.0.lock().map_err(|_| AppError::internal())?;
        if inner.status.phase != "ready" {
            return Err(error("UPDATE_NOT_READY", "请先完成更新下载"));
        }
        let update = inner.available.clone().ok_or_else(AppError::internal)?;
        let package = inner.package.take().ok_or_else(AppError::internal)?;
        inner.status.phase = "installing".into();
        (update, package)
    };
    let result = tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let bytes = std::fs::read(&package.path)
            .map_err(|_| error("UPDATE_IO_FAILED", "更新缓存已被清理，请重新下载"))?;
        update
            .verify(&bytes)
            .map_err(|_| error("UPDATE_SIGNATURE_INVALID", "更新签名无效，未启动安装"))?;
        let desktop = app.state::<DesktopState>();
        let api_was_enabled = desktop.application.status().api_enabled;
        desktop.application.enable_api(false);
        let install = (|| {
            for account in desktop.application.accounts_local()? {
                if desktop.repository.has_pending_tasks(&account.id)? {
                    return Err(error(
                        "UPDATE_TASKS_ACTIVE",
                        "请先完成或取消待处理任务，再安装更新",
                    ));
                }
            }
            desktop.native_provider.disconnect(None)?;
            desktop.application.clear_live_filehelper_local()?;
            update.install(bytes).map_err(|_| {
                error(
                    "UPDATE_INSTALL_FAILED",
                    "无法启动更新安装程序，请查看发布页",
                )
            })
        })();
        if install.is_err() {
            desktop.application.enable_api(api_was_enabled);
        }
        install
    })
    .await
    .map_err(|_| AppError::internal())?;
    match result {
        Ok(()) => state.status(),
        Err(e) => state.failed(e),
    }
}
#[tauri::command]
pub(super) fn open_release_page() -> AppResult<()> {
    let url = release_page()?;
    #[cfg(windows)]
    {
        use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SHOW_WINDOW_CMD};
        use windows::core::PCWSTR;
        let verb: Vec<_> = "open\0".encode_utf16().collect();
        let address: Vec<_> = url.as_str().encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(address.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SHOW_WINDOW_CMD(1),
            )
        };
        if result.0 as isize <= 32 {
            return Err(error("UPDATE_PAGE_FAILED", "无法打开系统浏览器"));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        Err(error(
            "UNSUPPORTED_PLATFORM",
            "此更新入口用于Windows桌面应用",
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_cache_cleanup_keeps_unrelated_files_and_rejects_active_downloads() {
        let directory =
            std::env::temp_dir().join(format!("werc-owned-update-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let package = directory.join(format!("{}.package", uuid::Uuid::new_v4()));
        std::fs::write(&package, b"owned test package").unwrap();
        let unrelated = directory.join("notes.txt");
        std::fs::write(&unrelated, b"owned unrelated").unwrap();
        let state = Updates::default();
        state.0.lock().unwrap().status.phase = "downloading".into();
        assert_eq!(state.clear_cache(&directory).unwrap_err().code, "BUSY");
        assert!(package.exists());
        state.0.lock().unwrap().status.phase = "available".into();
        assert_eq!(state.clear_cache(&directory).unwrap(), 1);
        assert!(unrelated.exists());
        assert!(!package.exists());
        std::fs::remove_file(&unrelated).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }
    #[test]
    fn update_addresses_require_https_without_credentials() {
        assert!(https("https://example.com/updates/latest.json").is_ok());
        for value in [
            "http://example.com/update",
            "file:///update",
            "https://user:pass@example.com/update",
            "https://example.com/update#fragment",
            "",
        ] {
            assert!(https(value).is_err(), "{value}");
        }
    }
    #[test]
    fn unconfigured_update_does_not_claim_up_to_date() {
        let state = Updates::default();
        let status = state.status().unwrap();
        assert_eq!(status.phase, "idle");
        let failure = state
            .failed(error("UPDATE_NOT_CONFIGURED", "尚未配置"))
            .unwrap();
        assert_eq!(failure.phase, "failed");
        assert_eq!(failure.error.unwrap().code, "UPDATE_NOT_CONFIGURED");
    }
}
