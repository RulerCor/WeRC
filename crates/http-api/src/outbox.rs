use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scope {
    account_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct History {
    account_id: String,
    before: Option<i64>,
    #[serde(default = "default_limit")]
    limit: u32,
}
pub(super) async fn history(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(q): Query<History>,
) -> Result<Json<werc_domain::outbox::SendPage>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let auth = secret.clone();
    let account = q.account_id.clone();
    let data = local_work(state.application.clone(), move |app| {
        app.send_history_page(&app.authenticate(&auth)?, &q.account_id, q.before, q.limit)
    })
    .await?;
    read_scope(&state.application.authenticate(&secret)?, &account)?;
    Ok(Json(data))
}
fn read_scope(p: &Principal, account: &str) -> Result<(), ApiError> {
    p.require("operations:read")?;
    p.require_account(account)?;
    Ok(())
}
pub(super) async fn list(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(scope): Query<Scope>,
) -> Result<Json<serde_json::Value>, ApiError> {
    read_scope(&p, &scope.account_id)?;
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = scope.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.sends(&p, &scope.account_id)
    })
    .await?;
    read_scope(&application.authenticate(&secret)?, &account)?;
    Ok(Json(json!({"data":data})))
}
pub(super) async fn status(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(scope): Query<Scope>,
) -> Result<Json<serde_json::Value>, ApiError> {
    read_scope(&p, &scope.account_id)?;
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = scope.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.send_status(&p, &scope.account_id, &id)
    })
    .await?;
    read_scope(&application.authenticate(&secret)?, &account)?;
    Ok(Json(json!({"data":data})))
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(scope): Query<Scope>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = scope.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.cancel_send(&p, &scope.account_id, &id)
    })
    .await?;
    let p = application.authenticate(&secret)?;
    p.require(match data.metadata.schema_version {
        9 | 10 | 11 | 12 => "moments:publish",
        4 => "messages:send_file",
        5 => "messages:send_video",
        6 => "messages:send_emoticon",
        3 => "messages:send_image",
        2 => "messages:send_voice",
        _ => "messages:send",
    })?;
    p.require("operations:create")?;
    p.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}

pub(super) async fn local_record(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(scope): Query<Scope>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    read_scope(&state.application.authenticate(&secret)?, &scope.account_id)?;
    let application = state.application.clone();
    let account = scope.account_id.clone();
    let auth = secret.clone();
    let data = local_work(state.application, move |app| {
        app.send_local_record_status(&app.authenticate(&auth)?, &scope.account_id, &id)
    })
    .await?;
    read_scope(&application.authenticate(&secret)?, &account)?;
    Ok(Json(json!({"data":data})))
}
