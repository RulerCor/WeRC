use super::*;
use werc_domain::wechat_settings::SettingTarget;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountInput {
    pub account_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TargetInput {
    pub account_id: String,
    pub target: SettingTarget,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApplyInput {
    pub account_id: String,
    pub target: SettingTarget,
    pub idempotency_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChangesInput {
    pub account_id: String,
    pub before: Option<i64>,
    #[serde(default = "page_limit")]
    pub limit: u32,
}
fn page_limit() -> u32 {
    20
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RestoreInput {
    pub account_id: String,
    pub idempotency_key: String,
}

async fn work<T: serde::Serialize + Send + 'static>(
    state: ApiState,
    headers: HeaderMap,
    action: impl FnOnce(&Application, &Principal) -> werc_domain::AppResult<T> + Send + 'static,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    state.application.authenticate(&secret)?;
    let auth = secret.clone();
    let app = state.application.clone();
    let result = local_work(app, move |app| {
        let p = app.authenticate(&auth)?;
        action(app, &p)
    })
    .await?;
    state.application.authenticate(&secret)?;
    Ok(Json(json!({"data":result})))
}
pub(super) async fn capabilities(
    State(s): State<ApiState>,
    h: HeaderMap,
    Query(q): Query<AccountInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    work(s, h, move |a, p| {
        a.wechat_settings_capabilities(p, &q.account_id)
    })
    .await
}
pub(super) async fn read(
    State(s): State<ApiState>,
    h: HeaderMap,
    Json(q): Json<AccountInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    work(s, h, move |a, p| a.wechat_settings_read(p, &q.account_id)).await
}
pub(super) async fn plan(
    State(s): State<ApiState>,
    h: HeaderMap,
    Json(q): Json<TargetInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    work(s, h, move |a, p| {
        a.wechat_settings_plan(p, &q.account_id, &q.target)
    })
    .await
}
pub(super) async fn apply(
    State(s): State<ApiState>,
    h: HeaderMap,
    Json(q): Json<ApplyInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    work(s, h, move |a, p| {
        a.wechat_settings_apply(p, &q.account_id, &q.target, &q.idempotency_key)
    })
    .await
}
pub(super) async fn changes(
    State(s): State<ApiState>,
    h: HeaderMap,
    Query(q): Query<ChangesInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    work(s, h, move |a, p| {
        let rows = a.wechat_settings_changes(p, &q.account_id, q.before, q.limit)?;
        Ok(changes_value(rows))
    })
    .await
}
pub(super) fn changes_value(
    rows: Vec<(i64, werc_domain::wechat_settings::SettingsChange)>,
) -> serde_json::Value {
    let next = rows.last().map(|r| r.0);
    json!({"items":rows.into_iter().map(|(position,change)|json!({"position":position,"change":change})).collect::<Vec<_>>(),"next_cursor":next})
}
pub(super) async fn restore(
    State(s): State<ApiState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(q): Json<RestoreInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    work(s, h, move |a, p| {
        a.wechat_settings_restore(p, &q.account_id, &id, &q.idempotency_key)
    })
    .await
}
