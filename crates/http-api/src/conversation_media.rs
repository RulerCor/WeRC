use super::*;
use werc_application::conversation_media::{
    ConversationMediaAsset, ConversationMediaRef, ConversationMediaSelection,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Save {
    account_id: String,
    selection: ConversationMediaSelection,
    forward_index: Option<usize>,
    forward_path: Option<Vec<usize>>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    account_id: String,
    reference: ConversationMediaRef,
}
fn access(p: &Principal, account: &str, scope: &str) -> Result<(), AppError> {
    p.require("messages:read")?;
    p.require(scope)?;
    p.require_account(account)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cancel {
    account_id: String,
    request_id: String,
}
fn key(p: &Principal, account: &str, id: &str) -> Result<String, AppError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效"));
    }
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("conversation-media", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn save(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Save>,
) -> Result<Json<ConversationMediaAsset>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id, "media:write")?;
    let request = key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let result = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, "media:write")?;
        app.save_conversation_media_path_local(
            &input.account_id,
            &input.selection,
            input.forward_index,
            input.forward_path.as_deref(),
            &request,
        )
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, "media:write")?;
    Ok(Json(result))
}
pub(super) async fn content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Saved>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    access(
        &state.application.authenticate(&secret)?,
        &input.account_id,
        "media:read",
    )?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, "media:read")?;
        app.conversation_media_content_local(&input.account_id, &input.reference)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, "media:read")?;
    Ok((
        [
            (header::CONTENT_TYPE, output.mime),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        output.file.bytes.to_vec(),
    )
        .into_response())
}
pub(super) async fn delete(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Saved>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    access(
        &state.application.authenticate(&secret)?,
        &input.account_id,
        "media:write",
    )?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, "media:write")?;
        app.delete_conversation_media_local(&input.account_id, &input.reference)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, "media:write")?;
    Ok(Json(json!({"deleted":true,"sent":false})))
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Cancel>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let p = state.application.authenticate(bearer(&headers)?)?;
    access(&p, &input.account_id, "media:write")?;
    state
        .application
        .cancel_browse_local(&key(&p, &input.account_id, &input.request_id)?)?;
    Ok(Json(json!({"cancel_requested":true})))
}
