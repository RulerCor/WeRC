use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LiveInput {
    account_id: String,
    view_id: String,
    local_id: String,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    account_id: String,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Account {
    account_id: String,
}
fn access(p: &Principal, account: &str, scope: &str) -> Result<(), AppError> {
    p.require("messages:read")?;
    p.require(scope)?;
    p.require_account(account)
}
fn request(p: &Principal, account: &str, id: &str) -> Result<String, AppError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效"));
    }
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("emoticon-cache", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn live_save(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<LiveInput>,
) -> Result<Json<werc_domain::file::FileAsset>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let principal = state.application.authenticate(&secret)?;
    access(&principal, &input.account_id, "media:write")?;
    let request = request(&principal, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let asset = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, "media:write")?;
        app.save_live_emoticon_local(&input.account_id, &input.view_id, &input.local_id, &request)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, "media:write")?;
    Ok(Json(asset))
}
pub(super) async fn receive(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(position): Path<i64>,
    Json(input): Json<Request>,
) -> Result<Json<werc_domain::file::FileAsset>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let principal = state.application.authenticate(&secret)?;
    access(&principal, &input.account_id, "media:write")?;
    let request = request(&principal, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let asset = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, "media:write")?;
        app.receive_emoticon_local(&input.account_id, position, &request)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, "media:write")?;
    Ok(Json(asset))
}
pub(super) async fn content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(position): Path<i64>,
    Query(input): Query<Account>,
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
    let file = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, "media:read")?;
        app.received_emoticon_content_local(&input.account_id, position)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, "media:read")?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                if file.file_name.ends_with(".png") {
                    "image/png"
                } else {
                    "image/gif"
                },
            ),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        file.bytes.to_vec(),
    )
        .into_response())
}
pub(super) async fn delete(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(position): Path<i64>,
    Query(input): Query<Account>,
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
        app.delete_received_emoticon_local(&input.account_id, position)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, "media:write")?;
    Ok(Json(json!({"deleted":true,"sent":false})))
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Request>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let p = state.application.authenticate(bearer(&headers)?)?;
    access(&p, &input.account_id, "media:write")?;
    state
        .application
        .cancel_browse_local(&request(&p, &input.account_id, &input.request_id)?)?;
    Ok(Json(json!({"cancel_requested":true})))
}
