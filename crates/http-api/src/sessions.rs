use super::*;
use werc_application::live_sessions::LiveSessionPage;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    view_id: String,
    cursor: Option<String>,
    search_query: Option<String>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cancel {
    account_id: String,
    request_id: String,
}
fn authorize(p: &Principal, account: &str) -> Result<(), ApiError> {
    p.require("sessions:read")?;
    p.require_account(account)?;
    Ok(())
}
fn request_key(p: &Principal, account: &str, id: &str) -> Result<String, ApiError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效").into());
    }
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("live-sessions", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn page(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<LiveSessionPage>, ApiError> {
    authorize(&p, &input.account_id)?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account_id.clone();
    let page = local_work(state.application, move |app| {
        app.live_sessions_search_local(
            &input.account_id,
            &input.view_id,
            input.cursor.as_deref(),
            input.search_query.as_deref(),
            &request,
        )
    })
    .await?;
    authorize(&app.authenticate(&secret)?, &account)?;
    Ok(Json(page))
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(input): Json<Cancel>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&p, &input.account_id)?;
    state.application.cancel_browse_local(&request_key(
        &p,
        &input.account_id,
        &input.request_id,
    )?)?;
    Ok(Json(json!({"cancel_requested":true})))
}
