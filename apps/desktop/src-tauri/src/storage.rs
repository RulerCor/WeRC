//! Fixed product locations only; protected documents are never cache cleanup targets.
use super::*;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Default, Clone)]
pub(super) struct StorageTasks(Arc<Mutex<Option<(String, Arc<AtomicBool>)>>>);
struct Lease {
    state: StorageTasks,
    id: String,
    cancel: Arc<AtomicBool>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut s) = self.state.0.lock() {
            if s.as_ref().is_some_and(|(id, _)| id == &self.id) {
                *s = None;
            }
        }
    }
}
impl Lease {
    fn check(&self) -> Result<(), AppError> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(AppError::new("QUERY_CANCELED", "存储扫描已取消"))
        } else {
            Ok(())
        }
    }
}
#[derive(Serialize)]
pub(super) struct Category {
    id: &'static str,
    label: &'static str,
    bytes: u64,
    files: u64,
    clearable: bool,
    note: &'static str,
}
#[derive(Serialize)]
pub(super) struct Usage {
    database: werc_infrastructure::storage_usage::StorageUsage,
    categories: Vec<Category>,
}
pub(super) fn ordinary(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        !meta.file_type().is_symlink() && meta.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        !meta.file_type().is_symlink()
    }
}
fn size(path: &Path, lease: &Lease) -> Result<(u64, u64), AppError> {
    lease.check()?;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(_) => return Err(AppError::new("STORAGE_IO", "无法读取存储位置")),
    };
    if !ordinary(&meta) {
        return Err(AppError::new("STORAGE_LINK", "存储目录经过链接，未扫描"));
    }
    if meta.is_file() {
        return Ok((meta.len(), 1));
    }
    let mut total = (0u64, 0u64);
    let mut pending = vec![(path.to_owned(), 0usize)];
    let mut visited = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        lease.check()?;
        for entry in
            std::fs::read_dir(dir).map_err(|_| AppError::new("STORAGE_IO", "无法扫描存储目录"))?
        {
            lease.check()?;
            visited += 1;
            if visited > 50000 {
                return Err(AppError::new(
                    "STORAGE_LIMIT",
                    "目录过大，请选择具体模型目录",
                ));
            }
            let entry = entry.map_err(|_| AppError::new("STORAGE_IO", "无法读取目录项"))?;
            let meta = std::fs::symlink_metadata(entry.path())
                .map_err(|_| AppError::new("STORAGE_IO", "无法读取目录项"))?;
            if !ordinary(&meta) {
                continue;
            }
            if meta.is_file() {
                total.0 = total.0.saturating_add(meta.len());
                total.1 += 1;
            } else if meta.is_dir() {
                if depth >= 16 {
                    return Err(AppError::new("STORAGE_LIMIT", "目录嵌套过深"));
                }
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    Ok(total)
}
#[tauri::command]
pub(super) async fn scan_storage(
    handle: tauri::AppHandle,
    state: State<'_, StorageTasks>,
    request_id: String,
) -> Result<Usage, AppError> {
    if uuid::Uuid::parse_str(&request_id).is_err() {
        return Err(AppError::new("INVALID_ARGUMENT", "扫描请求编号无效"));
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut guard = state.0.lock().map_err(|_| AppError::internal())?;
        if guard.is_some() {
            return Err(AppError::new("BUSY", "已有存储扫描正在运行"));
        }
        *guard = Some((request_id.clone(), cancel.clone()));
    }
    let lease = Lease {
        state: state.inner().clone(),
        id: request_id,
        cancel,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let data = handle.state::<storage_location::Locations>().data.clone();
        let database = handle.state::<DesktopState>().repository.storage_usage()?;
        let mut categories = vec![Category {
            id: "query",
            label: "可重建查询缓存",
            bytes: database.query_bytes,
            files: database.query_rows,
            clearable: true,
            note: "清理后首次查询会重新建立索引；保留历史快照。",
        }];
        let logs = handle.state::<diagnostics::Diagnostics>().storage(
            handle
                .state::<preferences::PreferencesStore>()
                .diagnostics_logging(),
        )?;
        categories.push(Category {
            id: "logs",
            label: "诊断日志",
            bytes: logs.bytes,
            files: logs.files as u64,
            clearable: true,
            note: "仅清理三个程序诊断日志。",
        });
        let update_directory = handle.path().app_cache_dir().map_err(|_| AppError::new("STORAGE_IO", "无法定位更新缓存"))?.join("updates");
        let (bytes, files) = size(&update_directory, &lease)?;
        categories.push(Category { id: "updates", label: "更新下载缓存", bytes, files, clearable: true, note: "仅清理本程序的临时更新包；清理后需重新下载。" });
        for (id, label, path, note) in [
            (
                "models",
                "用户转写模型",
                werc_wechat_win::asr::model_directory()?,
                "可在模型管理中单独删除或重新安装。",
            ),
            (
                "runtime",
                "随程序转写组件",
                werc_wechat_win::asr::root()?,
                "随程序提供的运行组件，不作为缓存清理。",
            ),
            (
                "protected",
                "受保护的本机资料",
                data.join("state.sqlite3"),
                "账号、密钥、消息、归档和媒体保留；数据文件包含缓存和SQLite可复用空间。",
            ),
        ] {
            let (bytes, files) = size(&path, &lease)?;
            categories.push(Category {
                id,
                label,
                bytes,
                files,
                clearable: false,
                note,
            });
        }
        lease.check()?;
        Ok(Usage {
            database,
            categories,
        })
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) fn cancel_storage_scan(
    state: State<'_, StorageTasks>,
    request_id: String,
) -> Result<(), AppError> {
    if let Some((id, cancel)) = state.0.lock().map_err(|_| AppError::internal())?.as_ref() {
        if id == &request_id {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}
#[tauri::command]
pub(super) async fn clear_storage_category(
    handle: tauri::AppHandle,
    category: String,
) -> Result<usize, AppError> {
    tauri::async_runtime::spawn_blocking(move || match category.as_str() {
        "updates" => {
            let directory = handle.path().app_cache_dir().map_err(|_| AppError::new("STORAGE_IO", "无法定位更新缓存"))?.join("updates");
            let meta = std::fs::symlink_metadata(&directory);
            if meta.as_ref().is_ok_and(|m| !ordinary(m)) { return Err(AppError::new("STORAGE_LINK", "更新缓存目录经过链接，未清理")); }
            handle.state::<updates::Updates>().clear_cache(&directory)
        }
        "logs" => {
            handle.state::<diagnostics::Diagnostics>().clear()?;
            Ok(0)
        }
        "query" => {
            let app = &handle.state::<DesktopState>().application;
            let mut n = 0;
            for a in app.accounts_local()? {
                n += app.clear_browse_cache_local(&a.id)?;
            }
            Ok(n)
        }
        "expired_uploads" => {
            let repo = &handle.state::<DesktopState>().repository;
            Ok(repo.purge_expired_audio()?
                + repo.purge_expired_images()?
                + repo.purge_expired_files()?
                + repo.purge_expired_videos()?)
        }
        _ => Err(AppError::new("PROTECTED_DATA", "此类资料不能作为缓存清理")),
    })
    .await
    .map_err(|_| AppError::internal())?
}
