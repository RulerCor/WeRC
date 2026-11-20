//! Best-effort local display, independent of native ACK and Bot delivery.
use super::*;
use std::time::{Duration, Instant};
use tauri::{PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
#[derive(Clone, Serialize)]
pub(super) struct Navigation {
    pub account: String,
    pub conversation: String,
    pub position: i64,
    pub page: String,
}
#[derive(Clone, Serialize)]
pub(super) struct Notice {
    id: String,
    title: String,
    body: String,
}
struct Pending {
    notice: Notice,
    navigation: Navigation,
    expires: Instant,
}
#[derive(Default)]
pub(super) struct Notifications {
    render: Mutex<()>,
    pending: Mutex<Option<Pending>>,
    navigation: Mutex<Option<Navigation>>,
    last: Mutex<Option<String>>,
}
fn display(
    handle: &tauri::AppHandle,
    notice: Notice,
    navigation: Navigation,
) -> Result<(), AppError> {
    let state = handle.state::<Notifications>();
    {
        let mut last = state.last.lock().map_err(|_| AppError::internal())?;
        if last.as_deref() == Some(&notice.id) {
            return Ok(());
        }
        *last = Some(notice.id.clone());
    }
    *state.pending.lock().map_err(|_| AppError::internal())? = Some(Pending {
        notice,
        navigation,
        expires: Instant::now() + Duration::from_secs(10),
    });
    let app = handle.clone();
    // WebView2 creation must not execute in the main-thread IPC/event callback.
    // Serialize builders so concurrent message/task notices share one window.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Notifications>();
        let Ok(_render) = state.render.lock() else {
            return;
        };
        let window = if let Some(w) = app.get_webview_window("notification") {
            w
        } else {
            let Ok(w) = WebviewWindowBuilder::new(
                &app,
                "notification",
                WebviewUrl::App("index.html?notification=1".into()),
            )
            .title("WeRC 新通知")
            .inner_size(360., 150.)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(false)
            .build() else {
                return;
            };
            w
        };
        if !state
            .pending
            .lock()
            .is_ok_and(|p| p.as_ref().is_some_and(|n| Instant::now() < n.expires))
        {
            return;
        }
        if let Some(monitor) = app
            .get_webview_window("main")
            .and_then(|w| w.current_monitor().ok().flatten())
            .or_else(|| window.current_monitor().ok().flatten())
        {
            let area = monitor.work_area();
            let scale = monitor.scale_factor();
            let width = ((360. * scale) as u32).min(area.size.width);
            let height = ((150. * scale) as u32).min(area.size.height);
            let _ = window.set_size(PhysicalSize::new(width, height));
            let _ = window.set_position(PhysicalPosition::new(
                area.position
                    .x
                    .saturating_add(area.size.width.saturating_sub(width + 12) as i32),
                area.position
                    .y
                    .saturating_add(area.size.height.saturating_sub(height + 12) as i32),
            ));
        }
        let _ = window.show();
    });
    Ok(())
}
pub(super) fn received(handle: &tauri::AppHandle, application: &Application, account: &str) {
    let (enabled, preview, _) = handle
        .state::<preferences::PreferencesStore>()
        .notifications();
    if !enabled {
        return;
    }
    let Ok(mut rows) = application.received_texts_local(account, -1, 1) else {
        return;
    };
    let Some(row) = rows.pop() else { return };
    let p = &row.payload;
    if p.route.as_ref().is_none_or(|r| r.sender == p.sender_id) {
        return;
    }
    let unlocked = handle.state::<app_lock::AppLock>().require_open().is_ok();
    let body = if preview && unlocked {
        if p.message.message_type == 1 {
            p.message.text.chars().take(100).collect()
        } else {
            match p.message.message_type {
                3 => "[图片]",
                34 => "[语音]",
                43 | 62 => "[视频]",
                47 => "[表情]",
                49 => "[文件或应用消息]",
                _ => "[新消息]",
            }
            .into()
        }
    } else {
        "收到一条新消息，点击查看".into()
    };
    let _ = display(
        handle,
        Notice {
            id: p.id.clone(),
            title: "WeRC · 新消息".into(),
            body,
        },
        Navigation {
            account: account.into(),
            conversation: p.target_id.clone(),
            position: row.position,
            page: "history".into(),
        },
    );
}
pub(super) fn task_finished(handle: &tauri::AppHandle) {
    task_finished_at(handle, "audio");
}
pub(super) fn task_finished_at(handle: &tauri::AppHandle, page: &str) {
    if !handle
        .state::<preferences::PreferencesStore>()
        .notifications()
        .2
    {
        return;
    }
    let _ = display(
        handle,
        Notice {
            id: uuid::Uuid::new_v4().to_string(),
            title: "WeRC · 任务处理结束".into(),
            body: "点击查看任务结果".into(),
        },
        Navigation {
            account: String::new(),
            conversation: String::new(),
            position: 0,
            page: page.into(),
        },
    );
}
#[tauri::command]
pub(super) async fn desktop_notification(
    handle: tauri::AppHandle,
) -> Result<Option<Notice>, AppError> {
    let state = handle.state::<Notifications>();
    let mut guard = state.pending.lock().map_err(|_| AppError::internal())?;
    if guard.as_ref().is_some_and(|p| Instant::now() >= p.expires) {
        *guard = None;
    }
    let Some(p) = guard.as_ref() else {
        if let Some(w) = handle.get_webview_window("notification") {
            let _ = w.hide();
        }
        return Ok(None);
    };
    let mut n = p.notice.clone();
    if handle.state::<app_lock::AppLock>().require_open().is_err()
        || !handle
            .state::<preferences::PreferencesStore>()
            .notifications()
            .1
    {
        n.body = if p.navigation.page == "history" {
            "收到一条新消息，点击查看"
        } else {
            "点击查看任务结果"
        }
        .into();
    }
    Ok(Some(n))
}
#[tauri::command]
pub(super) async fn dismiss_desktop_notification(
    handle: tauri::AppHandle,
    id: String,
) -> Result<(), AppError> {
    let state = handle.state::<Notifications>();
    let mut guard = state.pending.lock().map_err(|_| AppError::internal())?;
    if guard.as_ref().is_some_and(|p| p.notice.id == id) {
        *guard = None;
        if let Some(w) = handle.get_webview_window("notification") {
            let _ = w.hide();
        }
    }
    Ok(())
}
#[tauri::command]
pub(super) async fn activate_desktop_notification(
    handle: tauri::AppHandle,
    id: String,
) -> Result<(), AppError> {
    let state = handle.state::<Notifications>();
    {
        let mut guard = state.pending.lock().map_err(|_| AppError::internal())?;
        if guard.as_ref().is_some_and(|p| p.notice.id == id) {
            let p = guard.take().ok_or_else(AppError::internal)?;
            *state.navigation.lock().map_err(|_| AppError::internal())? = Some(p.navigation);
        }
    }
    if let Some(w) = handle.get_webview_window("notification") {
        let _ = w.hide();
    }
    if let Some(w) = handle.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    Ok(())
}
#[tauri::command]
pub(super) fn take_notification_navigation(
    state: State<'_, Notifications>,
) -> Result<Option<Navigation>, AppError> {
    Ok(state
        .navigation
        .lock()
        .map_err(|_| AppError::internal())?
        .take())
}

#[tauri::command]
pub(super) async fn test_desktop_notification(handle: tauri::AppHandle) -> Result<(), AppError> {
    display(
        &handle,
        Notice {
            id: uuid::Uuid::new_v4().to_string(),
            title: "WeRC · 自动化测试通知".into(),
            body: "自动化测试：本机通知显示正常".into(),
        },
        Navigation {
            account: String::new(),
            conversation: String::new(),
            position: 0,
            page: "history".into(),
        },
    )
}
