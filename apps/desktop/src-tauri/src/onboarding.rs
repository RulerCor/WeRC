use super::{DesktopState, run_local};
use tauri::{Manager, State};
use werc_domain::{
    AppError,
    onboarding::{SetupRequest, SetupStatus},
};

static FIXTURE: std::sync::Mutex<Option<(String, String, u32)>> = std::sync::Mutex::new(None);
#[tauri::command]
pub fn setup_fixture(
    app: tauri::AppHandle,
    executable: String,
    root: String,
    pid: u32,
) -> Result<(), AppError> {
    if app.config().identifier != "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "仅隔离测试可装配自有目标"));
    }
    let item = werc_wechat_win::discovery::inspect(&executable)?;
    if item.product_name != "WeRC synthetic memory target" || pid == 0 {
        return Err(AppError::new("INVALID_ARGUMENT", "需要自有合成目标"));
    }
    werc_wechat_win::data_sources::inspect_root(std::path::Path::new(&root))?;
    *FIXTURE.lock().map_err(|_| AppError::internal())? = Some((executable, root, pid));
    Ok(())
}
#[tauri::command]
pub async fn setup_discover(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    confirmed: bool,
    fixture: Option<String>,
    root: Option<String>,
) -> Result<serde_json::Value, AppError> {
    if !confirmed {
        return Err(AppError::new(
            "CONSENT_REQUIRED",
            "请确认检测安装和数据目录元数据",
        ));
    }
    if fixture.is_some() && app.config().identifier != "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "合成安装仅限隔离测试标识"));
    }
    let owned = if app.config().identifier == "local.werc.mon3trbot.smoke" {
        FIXTURE.lock().map_err(|_| AppError::internal())?.clone()
    } else {
        None
    };
    if app.config().identifier == "local.werc.mon3trbot.smoke" && owned.is_none() {
        return Err(AppError::new(
            "FIXTURE_REQUIRED",
            "隔离引导必须先装配自有目标，禁止自动扫描真实来源",
        ));
    }
    run_local(state,move |application| {
        if let Some((executable,root,pid))=owned {
            let mut item=application.inspect_client_local(&executable)?;item.process_ids=vec![pid];
            let catalog=werc_wechat_win::data_sources::inspect_root(std::path::Path::new(&root))?;
            return Ok(serde_json::json!({"clients":{"installations":[item],"warnings":[]},"catalog":catalog,"main_process_ids":[pid]}));
        }
        let report=if let Some(path)=fixture {werc_domain::connection::DiscoveryReport {installations:vec![application.inspect_client_local(&path)?],warnings:vec![]}} else {application.discover_clients_local()?};
        let catalog=match root {Some(path)=>werc_wechat_win::data_sources::inspect_root(std::path::Path::new(&path))?,None=>werc_wechat_win::data_sources::discover()?};
        let ids:Vec<_>=report.installations.iter().flat_map(|i|i.process_ids.iter().copied()).collect();
        Ok(serde_json::json!({"clients":report,"catalog":catalog,"main_process_ids":werc_wechat_win::setup::main_window_processes(&ids)}))
    }).await
}
#[tauri::command]
pub async fn setup_status(state: State<'_, DesktopState>) -> Result<SetupStatus, AppError> {
    run_local(state, |a| a.setup_status_local()).await
}
#[tauri::command]
pub async fn setup_start(
    state: State<'_, DesktopState>,
    request: SetupRequest,
    confirmed: bool,
) -> Result<SetupStatus, AppError> {
    run_local(state, move |a| a.setup_start_local(request, confirmed)).await
}
#[tauri::command]
pub async fn setup_step(
    state: State<'_, DesktopState>,
    id: String,
) -> Result<SetupStatus, AppError> {
    run_local(state, move |a| a.setup_step_local(&id)).await
}
#[tauri::command]
pub async fn setup_resume(
    state: State<'_, DesktopState>,
    id: String,
    confirmed: bool,
    skip: bool,
) -> Result<SetupStatus, AppError> {
    run_local(state, move |a| a.setup_resume_local(&id, confirmed, skip)).await
}
#[tauri::command]
pub fn setup_pause(state: State<'_, DesktopState>) -> Result<(), AppError> {
    state.application.setup_pause_local()
}
#[tauri::command]
pub fn setup_dismiss(state: State<'_, DesktopState>) -> Result<(), AppError> {
    state.application.setup_dismiss_local()
}
#[tauri::command]
pub fn setup_reset(state: State<'_, DesktopState>) -> Result<(), AppError> {
    state.application.setup_reset_local()
}
#[tauri::command]
pub fn runtime_location(app: tauri::AppHandle) -> Result<serde_json::Value, AppError> {
    Ok(
        serde_json::json!({"executable":std::env::current_exe().map_err(|_|AppError::internal())?,"identifier":app.config().identifier,"data_directory":app.state::<crate::storage_location::Locations>().data}),
    )
}
