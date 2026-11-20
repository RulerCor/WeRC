use super::*;
use werc_application::message_preservation::{PreservationStatus, PreservedPage};
#[tauri::command]
pub(super) async fn message_preservation_status(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<PreservationStatus, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.message_preservation_status_local(&account_id))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn configure_message_preservation(
    state: State<'_, DesktopState>,
    account_id: String,
    conversation: String,
    enabled: bool,
) -> Result<PreservationStatus, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.configure_message_preservation_local(&account_id, &conversation, enabled)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn preserved_messages(
    state: State<'_, DesktopState>,
    account_id: String,
    before: Option<i64>,
    conversation: Option<String>,
    status: Option<String>,
) -> Result<PreservedPage, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.preserved_messages_local(
            &account_id,
            before,
            conversation.as_deref(),
            status.as_deref(),
        )
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn step_message_preservation(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<PreservationStatus, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.step_message_preservation_local(&account_id))
        .await
        .map_err(|_| AppError::internal())?
}
