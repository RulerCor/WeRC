use super::*;
use werc_application::articles::{ArticleAction, ArticleExportFormat};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    request_id: String,
    operation: ArticleAction,
}
fn access(p: &Principal, account: &str, write: bool) -> Result<(), AppError> {
    p.require("sources:read")?;
    p.require_account(account)?;
    if write {
        p.require("operations:create")?;
    }
    Ok(())
}
pub(super) async fn action(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    let write = input.operation.writes();
    access(&p, &input.account_id, write)?;
    let request = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    let account = input.account_id.clone();
    let app = state.application.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        access(&app.authenticate(&auth)?, &input.account_id, write)?;
        app.article_action_local(&input.account_id, input.operation, &request)
    })
    .await?;
    access(&app.authenticate(&secret)?, &account, write)?;
    Ok(Json(output))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportInput {
    account_id: String,
    request_id: String,
    ids: Vec<String>,
    format: ArticleExportFormat,
}
pub(super) async fn export(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<ExportInput>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    access(&p, &input.account_id, false)?;
    p.require("exports:create")?;
    let request = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    let account = input.account_id.clone();
    let app = state.application.clone();
    let auth = secret.clone();
    let mime = input.format.mime();
    let output = local_work(state.application, move |app| {
        let p = app.authenticate(&auth)?;
        access(&p, &input.account_id, false)?;
        p.require("exports:create")?;
        app.export_articles_local(&input.account_id, &input.ids, input.format, &request)
    })
    .await?;
    let p = app.authenticate(&secret)?;
    access(&p, &account, false)?;
    p.require("exports:create")?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        output.bytes,
    )
        .into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CancelInput {
    account_id: String,
    request_id: String,
}
pub(super) async fn cancel(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Json(input): Json<CancelInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    access(&p, &input.account_id, false)?;
    let key = conversation::request_key(&p, &input.account_id, &input.request_id)?;
    state.application.cancel_browse_local(&key)?;
    Ok(Json(json!({"canceled":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JobInput {
    account_id: String,
    operation: werc_application::article_jobs::JobAction,
}
pub(super) async fn jobs(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<JobInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    let write = input.operation.writes();
    access(&p, &input.account_id, write)?;
    if write {
        p.require("exports:create")?;
    }
    let app = state.application.clone();
    let account = input.account_id.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        let p = app.authenticate(&auth)?;
        access(&p, &input.account_id, write)?;
        if write {
            p.require("exports:create")?;
        }
        app.article_job_action_local(&input.account_id, input.operation)
    })
    .await?;
    let p = app.authenticate(&secret)?;
    access(&p, &account, write)?;
    if write {
        p.require("exports:create")?;
    }
    Ok(Json(output))
}
