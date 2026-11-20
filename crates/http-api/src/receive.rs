use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryInput {
    account_id: String,
    #[serde(default = "latest")]
    after: i64,
    #[serde(default = "limit")]
    limit: u32,
}
fn latest() -> i64 {
    -1
}
fn limit() -> u32 {
    100
}
pub(super) async fn list(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(q): Query<QueryInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    p.require("messages:read")?;
    p.require_account(&q.account_id)?;
    let secret = bearer(&headers)?.to_owned();
    let account = q.account_id.clone();
    let application = state.application.clone();
    let data=local_work(state.application,move|app| {
        let rows=app.received_texts(&p,&q.account_id,q.after,q.limit)?;
        let next=rows.last().map_or(q.after.max(0),|row|row.position);
        Ok(json!({"data":rows,"next_cursor":next,"cursor":app.receive_cursor_local(&q.account_id)?,"health":app.receive_health_local(&q.account_id)?,"data_mode":"native_observations","historical_complete":false}))
    }).await?;
    let fresh = application.authenticate(&secret)?;
    fresh.require("messages:read")?;
    fresh.require_account(&account)?;
    Ok(Json(data))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImageQuery {
    account_id: String,
}
fn image_access(p: &Principal, account: &str, scope: &str) -> Result<(), AppError> {
    p.require("messages:read")?;
    p.require(scope)?;
    p.require_account(account)
}
pub(super) async fn image_download(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:write",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    let asset = local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:write")?;
        app.receive_image_file_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:write",
    )?;
    Ok(Json(json!({"data":asset,"sent":false})))
}
pub(super) async fn image_content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:read",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    let bytes = local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:read")?;
        app.received_image_preview_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:read",
    )?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes.to_vec(),
    )
        .into_response())
}

pub(super) async fn voice_content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:read",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    let bytes = local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:read")?;
        app.received_voice_preview_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:read",
    )?;
    Ok((
        [
            (header::CONTENT_TYPE, "audio/wav"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes.to_vec(),
    )
        .into_response())
}

pub(super) async fn file_download(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:write",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    let asset = local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:write")?;
        app.receive_file_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:write",
    )?;
    Ok(Json(json!({"data":asset,"sent":false})))
}

pub(super) async fn file_content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:read",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    let bytes = local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:read")?;
        app.received_file_content_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:read",
    )?;
    let disposition = format!(
        "attachment; filename*=UTF-8\'\'{}",
        bytes
            .file_name
            .as_bytes()
            .iter()
            .map(|b| format!("%{b:02X}"))
            .collect::<String>()
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes.bytes.to_vec(),
    )
        .into_response())
}

pub(super) async fn video_download(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:write",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    let asset = local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:write")?;
        app.receive_video_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:write",
    )?;
    Ok(Json(json!({"data":asset,"sent":false})))
}
pub(super) async fn video_content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:read",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    let bytes = local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:read")?;
        app.received_video_content_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:read",
    )?;
    Ok((
        [
            (header::CONTENT_TYPE, "video/mp4"),
            (header::CACHE_CONTROL, "no-store"),
            (header::ACCEPT_RANGES, "none"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes.to_vec(),
    )
        .into_response())
}

pub(super) async fn video_delete(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<ImageQuery>,
    Path(position): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    image_access(
        &state.application.authenticate(&secret)?,
        &q.account_id,
        "media:write",
    )?;
    let application = state.application.clone();
    let account = q.account_id.clone();
    let output_secret = secret.clone();
    local_work(state.application, move |app| {
        image_access(&app.authenticate(&secret)?, &q.account_id, "media:write")?;
        app.delete_received_video_local(&q.account_id, position)
    })
    .await?;
    image_access(
        &application.authenticate(&output_secret)?,
        &account,
        "media:write",
    )?;
    Ok(Json(json!({"deleted":true,"sent":false})))
}
