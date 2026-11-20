use super::*;
use werc_application::live_favorites::FavoritePage;
use werc_domain::favorites::FavoriteQuery;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    view_id: String,
    query: Option<FavoriteQuery>,
    cursor: Option<String>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Item {
    account_id: String,
    view_id: String,
    item_token: String,
    slot: Option<Vec<usize>>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cancel {
    account_id: String,
    request_id: String,
}
fn access(p: &Principal, account: &str) -> Result<(), AppError> {
    p.require("sources:read")?;
    p.require_account(account)
}
fn key(p: &Principal, account: &str, id: &str) -> Result<String, ApiError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效").into());
    }
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("live-favorites", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<FavoritePage>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id)?;
    let request = key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id)?;
        app.live_favorites_local(
            &input.account_id,
            &input.view_id,
            input.query,
            input.cursor.as_deref(),
            &request,
        )
    })
    .await?;
    access(&app.authenticate(&secret)?, &account)?;
    Ok(Json(output))
}
pub(super) async fn content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Item>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id)?;
    p.require("media:read")?;
    let request = key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        let p = app.authenticate(&auth)?;
        access(&p, &input.account_id)?;
        p.require("media:read")?;
        app.favorite_media_local(
            &input.account_id,
            &input.view_id,
            &input.item_token,
            &input
                .slot
                .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "请选择收藏媒体"))?,
            &request,
        )
    })
    .await?;
    let p = app.authenticate(&secret)?;
    access(&p, &account)?;
    p.require("media:read")?;
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
pub(super) async fn copy(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Item>,
) -> Result<Json<werc_application::library::LibraryItem>, ApiError> {
    if input.slot.is_some() {
        return Err(AppError::new("INVALID_ARGUMENT", "复制正文不接受媒体选择").into());
    }
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id)?;
    p.require("exports:create")?;
    let request = key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        let p = app.authenticate(&auth)?;
        access(&p, &input.account_id)?;
        p.require("exports:create")?;
        app.copy_favorite_local(
            &input.account_id,
            &input.view_id,
            &input.item_token,
            &request,
        )
    })
    .await?;
    let p = app.authenticate(&secret)?;
    access(&p, &account)?;
    p.require("exports:create")?;
    Ok(Json(output))
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Cancel>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let p = state.application.authenticate(bearer(&headers)?)?;
    access(&p, &input.account_id)?;
    state
        .application
        .cancel_browse_local(&key(&p, &input.account_id, &input.request_id)?)?;
    Ok(Json(json!({"cancel_requested":true})))
}
