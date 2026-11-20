use super::*;
use werc_application::{live_export::LiveExportFormat, live_export::LiveExportSelection};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    view_id: String,
    selection: LiveExportSelection,
    format: LiveExportFormat,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cancel {
    account_id: String,
    request_id: String,
}
fn authorize(p: &Principal, account: &str) -> Result<(), ApiError> {
    p.require("messages:read")?;
    p.require("exports:create")?;
    p.require_account(account)?;
    Ok(())
}
fn key(p: &Principal, account: &str, id: &str) -> Result<String, ApiError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "导出标识格式无效").into());
    }
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("live-message-export", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn create(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<axum::response::Response, ApiError> {
    authorize(&p, &input.account_id)?;
    let request = key(&p, &input.account_id, &input.request_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account_id.clone();
    let format = input.format;
    let output = local_work(state.application, move |app| {
        app.export_live_messages_local(
            &input.account_id,
            &input.view_id,
            &input.selection,
            input.format,
            &request,
            Ok,
        )
    })
    .await?;
    authorize(&app.authenticate(&secret)?, &account)?;
    axum::response::Response::builder()
        .status(200)
        .header("Content-Type", format.content_type())
        .header(
            "Content-Disposition",
            format!(
                "attachment; filename=\"WeRC-messages.{}\"",
                format.extension()
            ),
        )
        .header("Cache-Control", "no-store")
        .header("X-WeRC-Records", output.summary.rows.to_string())
        .header("X-WeRC-Pages", output.summary.pages.to_string())
        .header("X-WeRC-Media", "0")
        .body(axum::body::Body::from(output.bytes.to_vec()))
        .map_err(|_| AppError::internal().into())
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(input): Json<Cancel>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&p, &input.account_id)?;
    state
        .application
        .cancel_browse_local(&key(&p, &input.account_id, &input.request_id)?)?;
    Ok(Json(json!({"cancel_requested":true})))
}
