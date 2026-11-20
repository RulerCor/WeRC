use super::*;
use werc_domain::browse::{BrowseRequest, ConversationPage, TimelinePage};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowseInput {
    account_id: String,
    request: BrowseRequest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CancelInput {
    account_id: String,
    request_id: String,
}
fn authorize(p: &Principal, account: &str) -> Result<(), ApiError> {
    p.require("messages:read")?;
    p.require("contacts:read")?;
    p.require("sessions:read")?;
    p.require_account(account)?;
    Ok(())
}
fn request_key(p: &Principal, account: &str, id: &str) -> Result<String, ApiError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效").into());
    }
    // API 调用者不能通过知道另一个令牌的 request_id 来取消它的工作。
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("browse", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn conversations(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(mut input): Json<BrowseInput>,
) -> Result<Json<ConversationPage>, ApiError> {
    authorize(&p, &input.account_id)?;
    input.request.request_id = request_key(&p, &input.account_id, &input.request.request_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account_id.clone();
    let page = local_work(state.application, move |app| {
        app.browse_conversations_local(&input.account_id, &input.request)
    })
    .await?;
    authorize(&app.authenticate(&secret)?, &account)?;
    Ok(Json(page))
}
pub(super) async fn messages(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(mut input): Json<BrowseInput>,
) -> Result<Json<TimelinePage>, ApiError> {
    authorize(&p, &input.account_id)?;
    input.request.request_id = request_key(&p, &input.account_id, &input.request.request_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account_id.clone();
    let page = local_work(state.application, move |app| {
        app.browse_messages_local(&input.account_id, &input.request)
    })
    .await?;
    authorize(&app.authenticate(&secret)?, &account)?;
    Ok(Json(page))
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(input): Json<CancelInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    authorize(&p, &input.account_id)?;
    state.application.cancel_browse_local(&request_key(
        &p,
        &input.account_id,
        &input.request_id,
    )?)?;
    Ok(Json(json!({"cancel_requested":true})))
}
