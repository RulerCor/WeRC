use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Clone, Serialize)]
pub(super) struct Progress {
    pub phase: String,
    pub copied: u64,
    pub total: u64,
    pub error: Option<String>,
}
pub(super) struct Startup {
    value: Mutex<Progress>,
    cancel: AtomicBool,
}
impl Default for Startup {
    fn default() -> Self {
        Self {
            value: Mutex::new(Progress {
                phase: "starting".into(),
                copied: 0,
                total: 0,
                error: None,
            }),
            cancel: AtomicBool::new(false),
        }
    }
}
impl Startup {
    #[cfg(test)]
    pub fn cancel_for_test(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
    pub fn update(&self, phase: &str, copied: u64, total: u64) {
        if let Ok(mut p) = self.value.lock() {
            p.phase = phase.into();
            p.copied = copied;
            p.total = total;
        }
    }
    pub fn check(&self) -> Result<(), AppError> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(AppError::new(
                "MIGRATION_CANCELED",
                "迁移已取消，继续使用原资料",
            ))
        } else {
            Ok(())
        }
    }
    pub fn finish(&self, result: Result<(), String>) {
        if let Ok(mut p) = self.value.lock() {
            match result {
                Ok(()) => p.phase = "ready".into(),
                Err(e) => {
                    p.phase = "failed".into();
                    p.error = Some(e)
                }
            }
        }
    }
}
#[tauri::command]
pub(super) fn startup_status(state: State<'_, Startup>) -> Result<Progress, AppError> {
    Ok(state
        .value
        .lock()
        .map_err(|_| AppError::internal())?
        .clone())
}
#[tauri::command]
pub(super) fn cancel_startup_migration(state: State<'_, Startup>) -> Result<(), AppError> {
    let mut p = state.value.lock().map_err(|_| AppError::internal())?;
    if p.phase == "migrating" {
        state.cancel.store(true, Ordering::SeqCst);
        p.phase = "canceling".into();
    }
    Ok(())
}
#[tauri::command]
pub(super) fn exit_failed_startup(
    handle: tauri::AppHandle,
    state: State<'_, Startup>,
) -> Result<(), AppError> {
    if state.value.lock().map_err(|_| AppError::internal())?.phase != "failed" {
        return Err(AppError::new("BUSY", "程序仍在启动"));
    }
    handle.exit(1);
    Ok(())
}
