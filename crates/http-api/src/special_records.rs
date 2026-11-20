use super::*;
use werc_domain::special_records::RecordQuery;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    view_id: String,
    query: Option<RecordQuery>,
    cursor: Option<String>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Export {
    account_id: String,
    view_id: String,
    query: RecordQuery,
    format: String,
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
        &serde_json::to_string(&("special-records", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<werc_application::live_special_records::SpecialRecordPage>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id)?;
    let request = key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id)?;
        app.live_special_records_local(
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
pub(super) async fn export(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Export>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id)?;
    p.require("exports:create")?;
    let request = key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let mime = match input.format.as_str() {
        "json" => "application/json",
        "csv" => "text/csv; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => return Err(AppError::new("INVALID_ARGUMENT", "记录导出格式无效").into()),
    };
    let disposition = format!("attachment; filename=\"WeRC-records.{}\"", input.format);
    let (bytes, count) = local_work(state.application, move |app| {
        let p = app.authenticate(&auth)?;
        access(&p, &input.account_id)?;
        p.require("exports:create")?;
        app.export_special_records_local(
            &input.account_id,
            &input.view_id,
            input.query,
            &request,
            &input.format,
            |bytes, count| Ok((zeroize::Zeroizing::new(bytes.to_vec()), count)),
        )
    })
    .await?;
    let p = app.authenticate(&secret)?;
    access(&p, &account)?;
    p.require("exports:create")?;
    Response::builder()
        .header("Content-Type", mime)
        .header("Content-Disposition", disposition)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-WeRC-Records", count.to_string())
        .body(axum::body::Body::from(bytes.to_vec()))
        .map_err(|_| AppError::internal().into())
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
