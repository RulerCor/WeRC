mod app_lock;
mod diagnostics;
mod gpu;
mod directory;
mod export_file;
mod export_jobs;
mod favorites;
mod library;
mod articles;
mod article_jobs;
mod models;
mod image_keys;
mod message_preservation;
mod moments;
mod native;
mod notifications;
mod onboarding;
mod outbox;
mod preferences;
mod save_dialog;
mod special_records;
mod startup;
mod profile_directory;
mod storage;
mod storage_location;
mod window_layout;
mod updates;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{
    Manager, State,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use werc_application::{Application, TokenValidity};
use werc_domain::connection::{ClientBinding, ClientInstallation, DiscoveryReport, KeyStatus};
use werc_domain::{Account, AppError, IssuedToken, TokenInfo};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;
use werc_wechat_win::managed_messages::ManagedWindowsWechatProvider;

struct DesktopState {
    native_provider: Arc<ManagedWindowsWechatProvider>,
    application: Arc<Application>,
    repository: Arc<SqliteRepository>,
    server: Mutex<ServerState>,
    _instance_lock: std::fs::File,
}
#[derive(Default)]
struct ServerState {
    address: Option<String>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}
#[derive(Serialize)]
struct Bootstrap {
    status: werc_application::RuntimeStatus,
    current_connection: werc_domain::current::CurrentConnection,
    accounts: Vec<Account>,
    tokens: Vec<TokenInfo>,
    capabilities: Vec<werc_domain::capability::Capability>,
    api_address: Option<String>,
    bindings: Vec<ClientBinding>,
    key_statuses: Vec<KeyStatus>,
    audio_assets: Vec<werc_domain::media::AudioAsset>,
    image_assets: Vec<werc_domain::image::ImageAsset>,
    file_assets: Vec<werc_domain::file::FileAsset>,
    operations: Vec<werc_domain::operation::Operation>,
    history_sources: Vec<werc_domain::history::HistorySource>,
}

#[tauri::command]
fn bootstrap(state: State<'_, DesktopState>) -> Result<Bootstrap, AppError> {
    Ok(Bootstrap {
        status: state.application.status(),
        current_connection: state.application.current_connection_local()?,
        history_sources: state
            .application
            .accounts_local()?
            .iter()
            .map(|a| state.application.history_sources_local(&a.id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect(),
        audio_assets: state
            .application
            .accounts_local()?
            .iter()
            .map(|a| state.application.audio_assets_local(&a.id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect(),
        image_assets: state
            .application
            .accounts_local()?
            .iter()
            .map(|a| state.application.image_assets_local(&a.id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect(),
        file_assets: state
            .application
            .accounts_local()?
            .iter()
            .map(|a| state.application.file_assets_local(&a.id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect(),
        operations: state
            .application
            .accounts_local()?
            .iter()
            .map(|a| state.application.operations_local(&a.id))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect(),
        accounts: state.application.accounts_local()?,
        tokens: state.application.tokens_local()?,
        capabilities: state.application.capabilities(),
        bindings: state.application.bindings_local()?,
        key_statuses: state
            .application
            .accounts_local()?
            .iter()
            .map(|a| state.application.key_status_local(&a.id))
            .collect::<Result<_, _>>()?,
        api_address: state
            .server
            .lock()
            .map_err(|_| AppError::internal())?
            .address
            .clone(),
    })
}
async fn run_local<T: Send + 'static>(
    state: State<'_, DesktopState>,
    operation: impl FnOnce(&Application) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let application = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || operation(&application))
        .await
        .map_err(|_| AppError::internal())?
}

#[tauri::command]
async fn discover_data_sources(
    app: tauri::AppHandle,
    root: Option<String>,
    confirmed: bool,
) -> Result<werc_domain::connection::DataSourceCatalog, AppError> {
    if !confirmed {
        return Err(AppError::new(
            "CONSENT_REQUIRED",
            "检测数据目录需要明确确认",
        ));
    }
    if app.config().identifier == "local.werc.mon3trbot.smoke" && root.is_none() {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不检测真实来源目录"));
    }
    tauri::async_runtime::spawn_blocking(move || match root {
        Some(path) => werc_wechat_win::data_sources::inspect_root(std::path::Path::new(&path)),
        None => werc_wechat_win::data_sources::discover(),
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
async fn import_history(
    state: State<'_, DesktopState>,
    account_id: String,
    path: String,
    key_hex: Option<String>,
    allow_plaintext: bool,
    confirmed: bool,
) -> Result<werc_domain::history::HistorySource, AppError> {
    run_local(state, move |app| {
        app.import_history_local(
            &account_id,
            werc_domain::history::HistoryReadRequest {
                path,
                key_hex,
                allow_plaintext,
            },
            confirmed,
        )
    })
    .await
}
#[tauri::command]
async fn browse_conversations(
    state: State<'_, DesktopState>,
    account_id: String,
    request: werc_domain::browse::BrowseRequest,
) -> Result<werc_domain::browse::ConversationPage, AppError> {
    run_local(state, move |app| {
        app.browse_conversations_local(&account_id, &request)
    })
    .await
}
#[tauri::command]
async fn browse_messages(
    state: State<'_, DesktopState>,
    account_id: String,
    request: werc_domain::browse::BrowseRequest,
) -> Result<werc_domain::browse::TimelinePage, AppError> {
    run_local(state, move |app| {
        app.browse_messages_local(&account_id, &request)
    })
    .await
}
#[tauri::command]
async fn clear_browse_cache(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<usize, AppError> {
    run_local(state, move |app| app.clear_browse_cache_local(&account_id)).await
}
#[tauri::command]
fn cancel_browse(state: State<'_, DesktopState>, request_id: String) -> Result<(), AppError> {
    state.application.cancel_browse_local(&request_id)
}

#[tauri::command]
async fn history_messages(
    state: State<'_, DesktopState>,
    account_id: String,
    query: werc_domain::history::HistoryQuery,
) -> Result<werc_domain::history::MessagePage, AppError> {
    run_local(state, move |app| {
        app.history_messages_local(&account_id, &query)
    })
    .await
}
#[tauri::command]
async fn history_contacts(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<werc_domain::history::HistoryContact>, AppError> {
    run_local(state, move |app| app.history_contacts_local(&account_id)).await
}
#[tauri::command]
async fn history_sessions(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<werc_domain::history::HistorySession>, AppError> {
    run_local(state, move |app| app.history_sessions_local(&account_id)).await
}
#[tauri::command]
fn remove_history(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<(), AppError> {
    state.application.remove_history_local(&account_id, &id)
}
#[tauri::command]
async fn save_history_export(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    query: werc_domain::history::HistoryQuery,
    format: String,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    run_local(state, move |app| {
        let destination = export_file::destination(&path, &downloads)?;
        let expected = match format.as_str() {
            "json" => "json",
            "csv" => "csv",
            "txt" => "txt",
            "jsonl" => "jsonl",
            "html" => "html",
            "md" => "md",
            "sql" => "sql",
            _ => return Err(AppError::new("INVALID_ARGUMENT", "导出格式未支持")),
        };
        if destination.extension().and_then(|s| s.to_str()) != Some(expected) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "扩展名必须与导出格式一致",
            ));
        }
        let output = app.export_history_local(&account_id, &query, &format)?;
        export_file::write_new(&destination, output.as_bytes())
    })
    .await
}

#[tauri::command]
async fn discover_clients(state: State<'_, DesktopState>) -> Result<DiscoveryReport, AppError> {
    run_local(state, |app| app.discover_clients_local()).await
}
#[tauri::command]
async fn inspect_client(
    state: State<'_, DesktopState>,
    path: String,
) -> Result<ClientInstallation, AppError> {
    run_local(state, move |app| app.inspect_client_local(&path)).await
}
#[tauri::command]
async fn bind_client(
    state: State<'_, DesktopState>,
    account_id: String,
    path: String,
) -> Result<ClientBinding, AppError> {
    run_local(state, move |app| app.bind_client_local(&account_id, &path)).await
}
#[tauri::command]
fn unbind_client(state: State<'_, DesktopState>, account_id: String) -> Result<(), AppError> {
    state.application.unbind_client_local(&account_id)
}
#[tauri::command]
async fn import_key(
    state: State<'_, DesktopState>,
    account_id: String,
    key_hex: String,
) -> Result<KeyStatus, AppError> {
    run_local(state, move |app| app.import_key_local(&account_id, key_hex)).await
}
#[tauri::command]
async fn acquire_database_key(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    process_id: u32,
    database: String,
    confirmed: bool,
    request_id: Option<String>,
    timeout_seconds: Option<u64>,
) -> Result<KeyStatus, AppError> {
    if let Some(request) = request_id {
        use tauri::Emitter;
        use std::sync::atomic::{AtomicU64, Ordering};
        return run_local(state, move |application| {
            let sequence = AtomicU64::new(0);
            let bytes_read = AtomicU64::new(0);
            let last_emit = AtomicU64::new(0);
            let emit = |phase: &str, bytes: u64| {
                let sequence = sequence.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app.emit("key-scan-progress", serde_json::json!({"request_id":request,"sequence":sequence,"phase":phase,"bytes_read":bytes}));
            };
            emit("scanning", 0);
            let result = application.acquire_database_key_cancellable_local(
                &account_id, process_id, &database, confirmed, &request, timeout_seconds.unwrap_or(30),
                &|bytes| {
                    bytes_read.store(bytes, Ordering::SeqCst);
                    if bytes.saturating_sub(last_emit.load(Ordering::SeqCst)) >= 4 * 1024 * 1024 {
                        last_emit.store(bytes, Ordering::SeqCst);
                        emit("scanning", bytes);
                    }
                },
            );
            emit(match &result { Ok(_) => "complete", Err(e) if e.code == "QUERY_CANCELED" => "canceled", Err(_) => "failed" }, bytes_read.load(Ordering::SeqCst));
            result
        }).await;
    }
    run_local(state, move |app| {
        app.acquire_database_key_local(&account_id, process_id, &database, confirmed)
    })
    .await
}
#[tauri::command]
fn remove_key(state: State<'_, DesktopState>, account_id: String) -> Result<(), AppError> {
    state.application.remove_key_local(&account_id)
}

#[tauri::command]
async fn upload_audio(
    state: State<'_, DesktopState>,
    account_id: String,
    bytes: Vec<u8>,
) -> Result<werc_domain::media::AudioAsset, AppError> {
    run_local(state, move |app| app.upload_audio_local(&account_id, bytes)).await
}
#[tauri::command]
async fn read_audio_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<Vec<u8>, AppError> {
    run_local(state, move |app| {
        Ok(app.audio_preview_local(&account_id, &id)?.to_vec())
    })
    .await
}
#[tauri::command]
fn delete_audio(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<(), AppError> {
    state.application.delete_audio_local(&account_id, &id)
}
#[tauri::command]
async fn upload_image(
    state: State<'_, DesktopState>,
    account_id: String,
    bytes: Vec<u8>,
) -> Result<werc_domain::image::ImageAsset, AppError> {
    run_local(state, move |app| app.upload_image_local(&account_id, bytes)).await
}
#[tauri::command]
async fn read_image_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<Vec<u8>, AppError> {
    run_local(state, move |app| {
        Ok(app.image_preview_local(&account_id, &id)?.to_vec())
    })
    .await
}
#[tauri::command]
fn delete_image(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<(), AppError> {
    state.application.delete_image_local(&account_id, &id)
}
#[tauri::command]
async fn upload_video(
    state: State<'_, DesktopState>,
    account_id: String,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<werc_domain::file::FileAsset, AppError> {
    run_local(state, move |app| {
        app.upload_video_local(&account_id, &file_name, bytes)
    })
    .await
}
#[tauri::command]
async fn read_video_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<werc_domain::outbox::VideoSendAsset, AppError> {
    run_local(state, move |app| app.video_preview_local(&account_id, &id)).await
}
#[tauri::command]
async fn preview_video_content(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<tauri::ipc::Response, AppError> {
    run_local(state, move |app| {
        let contents = app.video_contents_local(&account_id, &id)?;
        Ok(tauri::ipc::Response::new(contents.bytes.to_vec()))
    })
    .await
}
#[tauri::command]
async fn upload_emoticon(
    state: State<'_, DesktopState>,
    account_id: String,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<werc_domain::file::FileAsset, AppError> {
    run_local(state, move |app| {
        app.upload_emoticon_local(&account_id, &file_name, bytes)
    })
    .await
}
#[tauri::command]
async fn read_emoticon_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<werc_domain::outbox::EmoticonSendAsset, AppError> {
    run_local(state, move |app| {
        app.emoticon_preview_local(&account_id, &id)
    })
    .await
}
#[tauri::command]
async fn preview_emoticon_content(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<tauri::ipc::Response, AppError> {
    run_local(state, move |app| {
        let contents = app.emoticon_contents_local(&account_id, &id)?;
        Ok(tauri::ipc::Response::new(contents.bytes.to_vec()))
    })
    .await
}
#[tauri::command]
async fn upload_file(
    state: State<'_, DesktopState>,
    account_id: String,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<werc_domain::file::FileAsset, AppError> {
    run_local(state, move |app| {
        app.upload_file_local(&account_id, &file_name, bytes)
    })
    .await
}
#[tauri::command]
async fn read_file_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<werc_domain::file::FilePreview, AppError> {
    run_local(state, move |app| app.file_preview_local(&account_id, &id)).await
}
#[tauri::command]
fn delete_file(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<(), AppError> {
    state.application.delete_file_local(&account_id, &id)
}
#[tauri::command]
fn inspect_audio_task(
    state: State<'_, DesktopState>,
    account_id: String,
    asset_id: String,
    key: String,
) -> Result<werc_domain::operation::Operation, AppError> {
    state
        .application
        .inspect_audio_task_local(&account_id, &asset_id, &key)
}
#[tauri::command]
fn cancel_operation(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
) -> Result<(), AppError> {
    state.application.cancel_operation_local(&account_id, &id)
}

#[tauri::command]
fn select_current_connection(
    state: State<'_, DesktopState>,
    account_id: String,
    revision: String,
    confirmed: bool,
) -> Result<werc_domain::current::CurrentConnection, AppError> {
    state
        .application
        .select_current_connection_local(&account_id, &revision, confirmed)
}

#[tauri::command]
fn register_account(state: State<'_, DesktopState>, label: String) -> Result<Account, AppError> {
    state.application.register_account_local(&label)
}
#[tauri::command]
async fn remove_account(state: State<'_, DesktopState>, id: String) -> Result<(), AppError> {
    run_local(state, move |app| app.remove_account_local(&id)).await
}
#[tauri::command]
fn issue_token(
    state: State<'_, DesktopState>,
    label: String,
    account_ids: Vec<String>,
    scopes: Vec<String>,
    validity: TokenValidity,
) -> Result<IssuedToken, AppError> {
    state
        .application
        .issue_token_with_validity_local(&label, account_ids, scopes, validity)
}
#[tauri::command]
fn revoke_token(state: State<'_, DesktopState>, id: String) -> Result<(), AppError> {
    state.application.revoke_token_local(&id)
}
#[tauri::command]
async fn set_api_enabled(
    state: State<'_, DesktopState>,
    enabled: bool,
    port: u16,
) -> Result<(), AppError> {
    if !enabled {
        state.application.enable_api(false);
        let mut server = state.server.lock().map_err(|_| AppError::internal())?;
        if let Some(shutdown) = server.shutdown.take() {
            let _ = shutdown.send(());
        }
        server.address = None;
        return Ok(());
    }
    if port < 1024 {
        return Err(AppError::new("INVALID_ARGUMENT", "端口需为 1024–65535"));
    }
    if state
        .server
        .lock()
        .map_err(|_| AppError::internal())?
        .address
        .is_some()
    {
        return Err(AppError::new(
            "API_ALREADY_RUNNING",
            "请先停止 API 再更改端口",
        ));
    }
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|_| AppError::new("PORT_UNAVAILABLE", "端口无法监听；未终止占用端口的其他程序"))?;
    let address = format!(
        "http://{}",
        listener.local_addr().map_err(|_| AppError::internal())?
    );
    let (shutdown, receiver) = tokio::sync::oneshot::channel();
    {
        let mut server = state.server.lock().map_err(|_| AppError::internal())?;
        if server.address.is_some() {
            return Err(AppError::new("API_ALREADY_RUNNING", "API 已由另一请求启动"));
        }
        server.address = Some(address);
        server.shutdown = Some(shutdown);
    }
    state.application.enable_api(true);
    let application = state.application.clone();
    tauri::async_runtime::spawn(async move {
        if werc_http_api::serve(listener, application.clone(), receiver)
            .await
            .is_err()
        {
            application.enable_api(false);
        }
    });
    Ok(())
}

#[tauri::command]
fn finish_smoke(app: tauri::AppHandle, state: State<'_, DesktopState>) -> Result<(), AppError> {
    if !matches!(
        app.config().identifier.as_str(),
        "local.werc.mon3trbot.smoke" | "local.werc.mon3trbot.liveverify"
    ) {
        return Err(AppError::new("FORBIDDEN", "正常构建不提供隔离测试退出入口"));
    }
    state.application.enable_api(false);
    if let Some(shutdown) = state
        .server
        .lock()
        .map_err(|_| AppError::internal())?
        .shutdown
        .take()
    {
        let _ = shutdown.send(());
    }
    app.exit(0);
    Ok(())
}

fn initialize(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let directory = profile_directory::selected(std::env::args_os().skip(1))?
        .unwrap_or(app.path().app_local_data_dir()?);
    std::fs::create_dir_all(&directory)?;
    let instance_lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("instance.lock"))?;
    instance_lock.try_lock().map_err(|_| {
        AppError::new(
            "INSTANCE_ALREADY_RUNNING",
            "此配置已有程序运行，请从托盘打开",
        )
    })?;
    let locations = storage_location::Locations::open(directory, &app.state::<startup::Startup>())?;
    let directory = locations.data.clone();
    app.manage(locations);
    app.manage(preferences::PreferencesStore::open(directory.clone())?);
    app.manage(diagnostics::Diagnostics::new(directory.clone()));
    diagnostics::install_panic_hook(app.clone());
    diagnostics::event(app,"lifecycle","STARTING",None);
    app.manage(app_lock::AppLock::open(directory.clone())?);
    app.manage(notifications::Notifications::default());
    app.manage(models::Models::default());
    app.manage(storage::StorageTasks::default());
    let live_verification = app.config().identifier == "local.werc.mon3trbot.liveverify";
    let repository = Arc::new(if live_verification {
        SqliteRepository::in_memory()?
    } else {
        SqliteRepository::open(&directory.join("state.sqlite3"))?
    });
    repository.purge_expired_audio()?;
    repository.purge_expired_images()?;
    repository.purge_expired_files()?;
    repository.purge_expired_videos()?;
    let native_provider = match option_env!("WERC_SQLCIPHER_WORKER_SHA256") {
        Some(hash) => ManagedWindowsWechatProvider::with_live_reader(
            std::env::current_exe()?
                .parent()
                .ok_or_else(AppError::internal)?
                .join("werc-sqlcipher-worker.exe"),
            hash.into(),
        ),
        None => ManagedWindowsWechatProvider::default(),
    };
    let native_provider = Arc::new(if live_verification {
        native_provider.with_live_window_limit(120)?
    } else {
        native_provider
    });
    let application = Arc::new(Application::new(
        repository.clone(),
        native_provider.clone(),
    ));
    application.recover_local_tasks()?;
    application.recover_sends_local()?;
    export_jobs::recover(&application)?;
    article_jobs::start(application.clone(),app.path().download_dir()?,app.config().identifier != "local.werc.mon3trbot.smoke",app.clone());
    export_jobs::start(
        application.clone(),
        app.path().download_dir()?,
        app.config().identifier != "local.werc.mon3trbot.smoke",
        app.clone(),
    );
    app.manage(DesktopState {
        native_provider: native_provider.clone(),
        application: application.clone(),
        repository: repository.clone(),
        server: Mutex::new(ServerState::default()),
        _instance_lock: instance_lock,
    });
    let recovery_application = application.clone();
    let recovery_handle = app.clone();
    let recovery_provider = native_provider.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let worker = recovery_application.clone();
            let handle = recovery_handle.clone();
            let provider = recovery_provider.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Some(account) = worker.current_connection_local()?.account_id {
                    if provider.status().is_ok_and(|status| {
                        status.connected && status.account_id.as_deref() == Some(account.as_str())
                    }) {
                        // A disconnected account is expected between sessions. Connection attempts
                        // have their own diagnostics; keep backfill errors for actual failures.
                        if let Err(error) = worker.discover_backfill_sources_local(&account)
                            && error.code != "NATIVE_NOT_CONNECTED"
                        {
                            diagnostics::record(&handle, "backfill", &error);
                        }
                        if let Err(error) = worker.step_backfill_local(&account)
                            && error.code != "NATIVE_NOT_CONNECTED"
                        {
                            diagnostics::record(&handle, "backfill", &error);
                        }
                    }
                    if let Err(error) = worker.step_message_preservation_local(&account) {
                        diagnostics::record(&handle, "message-preservation", &error);
                    }
                    if let Err(error) = worker.step_moment_preservation_local(&account) {
                        diagnostics::record(&handle, "moment-preservation", &error);
                    }
                }
                Ok::<_, AppError>(())
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    let receiver_application = application.clone();
    let receiver_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let provider = native_provider.clone();
            let receiver = receiver_application.clone();
            let diagnostic_handle = receiver_handle.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if let Ok(status) = provider.status()
                    && let Some(account) = status.account_id
                {
                    for _ in 0..16 {
                        match receiver.receive_once_local(&account) {
                            Ok(progress) => {
                                let _ = provider.receive_progressed(&account);
                                if progress.stored {
                                    notifications::received(
                                        &diagnostic_handle,
                                        &receiver,
                                        &account,
                                    );
                                }
                                if !progress.acknowledged {
                                    break;
                                }
                            }
                            Err(e) => {
                                diagnostics::record(&diagnostic_handle, "receive", &e);
                                let _ = provider.receive_failed(&account, &e.code);
                                break;
                            }
                        }
                    }
                }
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });
    let task_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut cycle = 0u32;
        let mut receipt_cursor: Option<(String, Option<i64>)> = None;
        loop {
            let worker = application.clone();
            let handle = task_handle.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let started=std::time::Instant::now();
                let result = worker.run_local_task_once();
                if matches!(result, Ok(true)) {
                    notifications::task_finished(&handle);
                    diagnostics::event(&handle,"task","TASK_COMPLETED",Some(started.elapsed()));
                }
                if let Err(error) = &result {
                    diagnostics::record(&handle, "task", error);
                }
                result
            })
            .await;
            let sender = application.clone();
            let handle = task_handle.clone();
            let send_result = tauri::async_runtime::spawn_blocking(move || {
                let started=std::time::Instant::now();
                let result = sender.run_send_once_local();
                if matches!(result,Ok(true)){diagnostics::event(&handle,"send","SEND_PROCESSED",Some(started.elapsed()));}
                if let Err(error) = &result {
                    diagnostics::record(&handle, "send", error);
                }
                result
            })
            .await;
            cycle = cycle.wrapping_add(1);
            if matches!(send_result, Ok(Ok(false))) && cycle.is_multiple_of(5) {
                let reconciler = application.clone();
                let cursor = receipt_cursor.clone();
                if let Ok(Ok(next)) = tauri::async_runtime::spawn_blocking(move || {
                    let Some(account) = reconciler.current_connection_local()?.account_id else {
                        return Ok::<_, werc_domain::AppError>(None);
                    };
                    let before = cursor
                        .filter(|(a, _)| a == &account)
                        .and_then(|(_, position)| position);
                    let progress = reconciler.reconcile_receipt_page_local(&account, before)?;
                    Ok(Some((account, progress.next_cursor)))
                })
                .await
                {
                    receipt_cursor = next;
                }
            }
            if cycle.is_multiple_of(300) {
                let repository = repository.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    repository.purge_expired_audio()?;
                    repository.purge_expired_images()?;
                    repository.purge_expired_files()?;
                    repository.purge_expired_videos()
                })
                .await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    let show = MenuItem::with_id(app, "show", "打开 WeRC", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出并停止 API", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .tooltip("WeRC · API 随桌面运行")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "quit" => {
                app.state::<DesktopState>().application.enable_api(false);
                let _ = app.state::<DesktopState>().native_provider.disconnect(None);
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window_layout::fit(&window);
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::Updates::default())
        .setup(|app| {
            app.manage(startup::Startup::default());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let started = std::time::Instant::now();
                let result = initialize(&handle).map_err(|e| e.to_string());
                diagnostics::event(&handle,"lifecycle",if result.is_ok(){"STARTED"}else{"STARTUP_FAILED"},Some(started.elapsed()));
                let ready = result.is_ok();
                handle.state::<startup::Startup>().finish(result);
                if ready { updates::start_automatic(handle); }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "notification" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.try_state::<DesktopState>().is_some() {
                    preferences::on_close(window);
                } else if let Some(startup) = window.try_state::<startup::Startup>() {
                    let _ = startup::cancel_startup_migration(startup);
                }
            }
        })
        .invoke_handler(|invoke| {
            let command = invoke.message.command();
            let is_startup = matches!(
                command,
                "startup_status" | "cancel_startup_migration" | "exit_failed_startup"
            );
            if !is_startup
                && invoke
                    .message
                    .webview_ref()
                    .try_state::<DesktopState>()
                    .is_none()
            {
                invoke
                    .resolver
                    .reject(AppError::new("STARTUP_PENDING", "正在准备本机资料"));
                return true;
            }
            if !is_startup
                && !matches!(
                    command,
                    "desktop_notification"
                        | "dismiss_desktop_notification"
                        | "activate_desktop_notification"
                        | "app_lock_status"
                        | "unlock_desktop"
                        | "unlock_desktop_hello"
                        | "recover_app_lock"
                        | "app_lock_hello_available"
                        | "close_desktop"
                )
            {
                let result = invoke
                    .message
                    .webview_ref()
                    .state::<app_lock::AppLock>()
                    .require_open();
                if let Err(error) = result {
                    invoke.resolver.reject(error);
                    return true;
                }
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
                startup::startup_status,
                updates::update_status,
                updates::check_for_update,
                updates::download_update,
                updates::cancel_update_download,
                updates::install_update,
                updates::open_release_page,
                startup::cancel_startup_migration,
                startup::exit_failed_startup,
                storage_location::storage_location,
                storage_location::plan_storage_migration,
                storage_location::cancel_storage_migration,
                save_dialog::choose_storage_directory,
                storage::scan_storage,
                storage::cancel_storage_scan,
                storage::clear_storage_category,
                notifications::test_desktop_notification,
                notifications::desktop_notification,
                notifications::dismiss_desktop_notification,
                notifications::activate_desktop_notification,
                notifications::take_notification_navigation,
                gpu::cuda_detect,
                gpu::install_cuda_runtime,
                diagnostics::diagnostic_frontend_error,
                diagnostics::diagnostic_log_storage,
                diagnostics::clear_diagnostic_logs,
                diagnostics::diagnostic_report,
                diagnostics::export_diagnostic_report,
                app_lock::app_lock_status,
                app_lock::app_lock_activity,
                app_lock::lock_desktop,
                app_lock::unlock_desktop,
                app_lock::hello::unlock_desktop_hello,
                app_lock::hello::app_lock_hello_available,
                app_lock::configure_app_lock,
                app_lock::remove_app_lock,
                app_lock::recover_app_lock,
                preferences::desktop_preferences,
                preferences::save_desktop_preferences,
                preferences::close_desktop,
                models::model_catalog,
                models::install_model,
                models::cancel_model_install,
                models::remove_model,
                models::sherpa::sherpa_catalog,
                models::sherpa::import_sherpa_bundle,
                models::sherpa::remove_sherpa_bundle,
                save_dialog::choose_local_file,
                export_jobs::export_plans,
                export_jobs::save_export_plan,
                export_jobs::export_plan_action,
                export_jobs::export_runs,
                library::export_transcript,
                library::library_page,
                articles::article_action,
                articles::article_export,
                articles::article_job_action,
                library::library_save,
                library::library_delete,
                library::capture_start,
                library::capture_step,
                library::capture_list,
                library::capture_page,
                library::capture_media_page,
                library::capture_media_catalog,
                library::capture_social_report,
                library::capture_social_report_export,
                library::capture_timeline,
                library::capture_message_location,
                library::capture_records,
                library::capture_records_export,
                library::capture_media_preview,
                library::capture_original_preview,
                library::capture_media_audit,
                library::capture_media_progress,
                library::capture_media_step,
                library::capture_media_save,
                library::capture_media_export,
                library::capture_bundle_export,
                library::capture_transcript_progress,
                library::capture_transcript_step,
                library::capture_status,
                library::capture_edit_fields,
                library::capture_delete_preview,
                library::capture_add_preview,
                library::capture_edit_preview,
                library::capture_edit_apply,
                library::capture_edits,
                library::capture_report,
                library::capture_report_save,
                library::capture_export,
                library::save_visual_png,
                library::library_backup,
                library::asr_status,
                library::capture_names_status,
                library::capture_names_refresh,
                library::transcribe_audio,
                bootstrap,
                native::native_status,
                native::native_processes,
                native::native_live_prepare,
                native::native_live_continue,
                native::native_live_page,
                native::native_live_clear,
                native::native_live_contacts,
                native::native_contact_avatar,
                native::reveal_database_keys,
                native::native_current_profile,
                image_keys::image_key_status,
                image_keys::reveal_image_keys,
                image_keys::remove_image_keys,
                image_keys::verify_image_keys,
                image_keys::decode_image_sample,
                native::native_live_group_members,
                native::native_live_sessions,
                native::native_live_search,
                native::native_live_calendar,
                directory::save_directory_export,
                directory::save_live_message_export,
                directory::show_directory_export,
                save_dialog::choose_directory_export_path,
                save_dialog::choose_media_save_path,
                native::native_live_moments,
                moments::native_moment_overview,
                moments::native_moment_media,
                moments::moment_media_preview,
                moments::save_moment_media,
                moments::save_moment_bundle,
                message_preservation::message_preservation_status,
                message_preservation::configure_message_preservation,
                message_preservation::preserved_messages,
                message_preservation::step_message_preservation,
                moments::moment_preservation_status,
                moments::configure_moment_preservation,
                moments::preserved_moments,
                moments::save_preserved_moments,
                moments::save_moment_export,
                special_records::native_special_records,
                special_records::save_special_records,
                favorites::native_live_favorites,
                favorites::native_favorite_copy,
                favorites::native_favorite_preview,
                favorites::native_favorite_save,
                native::native_live_conversation,
                native::native_prepare_recipient,
                native::native_prepared_recipients,
                native::native_live_multi_conversation,
                native::native_live_multi_prepare,
                native::native_live_auto_prepare,
                native::native_live_multi_continue,
                native::native_live_multi_page,
                native::native_backfill_status,
                native::native_backfill_page,
                native::native_recovery_save,
                native::native_recovery_multi_save,
                native::native_recovery_list,
                native::native_recovery_page,
                native::native_connect,
                native::native_disconnect,
                native::native_messages,
                native::native_received_image_asset,
                native::native_received_video_asset,
                native::native_received_video_download,
                native::native_received_video_preview,
                native::native_received_video_delete,
                native::native_received_video_save,
                native::native_received_file_asset,
                native::native_received_file_info,
                native::native_received_file_download,
                native::native_received_file_save,
                native::native_live_file_save,
                native::native_live_file_info,
                native::native_live_voice_save,
                native::native_live_emoticon_save,
                native::native_conversation_media_save,
                native::native_conversation_media_content,
                native::native_conversation_media_delete,
                native::export_conversation_media,
                native::native_received_emoticon_asset,
                native::native_received_emoticon_read,
                native::export_emoticon_content,
                native::native_received_voice_asset,
                native::native_received_voice_preview,
                native::native_received_image_download,
                native::native_received_image_preview,
                native::native_fixture,
                outbox::list_sends,
                outbox::list_send_history,
                outbox::send_fixture,
                outbox::preview_send,
                outbox::create_text_draft,
                outbox::create_mention_draft,
                outbox::create_quote_draft,
                outbox::create_private_moment_draft,
                outbox::create_moment_draft,
                outbox::create_moment_action_draft,
                outbox::create_voice_draft,
                outbox::create_image_draft,
                outbox::create_file_draft,
                outbox::create_video_draft,
                outbox::create_emoticon_draft,
                outbox::approve_send,
                outbox::cancel_send,
                outbox::reconcile_send,
                onboarding::setup_discover,
                onboarding::setup_fixture,
                onboarding::setup_status,
                onboarding::setup_start,
                onboarding::setup_step,
                onboarding::setup_resume,
                onboarding::setup_pause,
                onboarding::setup_dismiss,
                onboarding::setup_reset,
                onboarding::runtime_location,
                finish_smoke,
                discover_data_sources,
                import_history,
                browse_conversations,
                browse_messages,
                clear_browse_cache,
                cancel_browse,
                history_messages,
                history_contacts,
                history_sessions,
                remove_history,
                save_history_export,
                discover_clients,
                inspect_client,
                bind_client,
                unbind_client,
                import_key,
                acquire_database_key,
                remove_key,
                upload_file,
                upload_video,
                read_video_preview,
                preview_video_content,
                upload_emoticon,
                read_emoticon_preview,
                preview_emoticon_content,
                read_file_preview,
                delete_file,
                upload_image,
                read_image_preview,
                delete_image,
                upload_audio,
                read_audio_preview,
                delete_audio,
                inspect_audio_task,
                cancel_operation,
                select_current_connection,
                register_account,
                remove_account,
                issue_token,
                revoke_token,
                set_api_enabled
            ];
            handler(invoke)
        })
        .build(tauri::generate_context!())
        .expect("WeRC 桌面初始化失败")
        .run(|handle,event| {
            if matches!(event,tauri::RunEvent::Exit){diagnostics::event(handle,"lifecycle","EXITED",None);}
        });
}
