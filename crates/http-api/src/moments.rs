use super::*;
use werc_application::live_moments::LiveMomentPage;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    account_id: String,
    view_id: String,
    cursor: Option<String>,
    query: Option<werc_domain::moments::MomentQuery>,
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cancel {
    account_id: String,
    request_id: String,
}
fn authorize(p: &Principal, account: &str) -> Result<(), ApiError> {
    p.require("moments:read")?;
    p.require_account(account)?;
    Ok(())
}
fn request_key(p: &Principal, account: &str, id: &str) -> Result<String, ApiError> {
    if !(16..=80).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效").into());
    }
    Ok(werc_application::token_digest(
        &serde_json::to_string(&("live-moments", &p.id, account, id))
            .map_err(|_| AppError::internal())?,
    ))
}
pub(super) async fn page(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<LiveMomentPage>, ApiError> {
    authorize(&p, &input.account_id)?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let secret = bearer(&headers)?.to_owned();
    let app = state.application.clone();
    let account = input.account_id.clone();
    let page = local_work(state.application, move |app| {
        app.query_live_moments_local(
            &input.account_id,
            &input.view_id,
            input.query,
            input.cursor.as_deref(),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OverviewInput {
    account_id: String,
    view_id: String,
    query: werc_domain::moments::MomentQuery,
    days: Vec<werc_domain::live::LiveCalendarDay>,
    request_id: String,
}
pub(super) async fn overview(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<OverviewInput>,
) -> Result<Json<werc_application::live_moments::MomentOverview>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    authorize(&p, &input.account_id)?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let account = input.account_id.clone();
    let app = state.application.clone();
    let auth = secret.clone();
    let output = local_work(state.application, move |app| {
        let p = app.authenticate(&auth)?;
        p.require("moments:read")?;
        p.require_account(&input.account_id)?;
        app.moment_overview_local(
            &input.account_id,
            &input.view_id,
            &input.query,
            &input.days,
            &request,
        )
    })
    .await?;
    authorize(&app.authenticate(&secret)?, &account)?;
    Ok(Json(output))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportInput {
    account_id: String,
    view_id: String,
    query: werc_domain::moments::MomentQuery,
    format: werc_application::live_moments::MomentExportFormat,
    request_id: String,
}
pub(super) async fn export(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<ExportInput>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    authorize(&p, &input.account_id)?;
    p.require("exports:create")?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let account = input.account_id.clone();
    let app = state.application.clone();
    let auth = secret.clone();
    let format = input.format;
    let output = local_work(state.application, move |app| {
        let p = app.authenticate(&auth)?;
        p.require("moments:read")?;
        p.require_account(&input.account_id)?;
        p.require("exports:create")?;
        app.export_live_moments_local(
            &input.account_id,
            &input.view_id,
            &input.query,
            format,
            &request,
            Ok,
        )
    })
    .await?;
    let p = app.authenticate(&secret)?;
    authorize(&p, &account)?;
    p.require("exports:create")?;
    Response::builder()
        .header("Content-Type", format.content_type())
        .header(
            "Content-Disposition",
            format!(
                "attachment; filename=\"WeRC-moments.{}\"",
                format.extension()
            ),
        )
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-WeRC-Records", output.summary.rows.to_string())
        .header(
            "X-WeRC-Missing-Media",
            output.summary.missing_media.to_string(),
        )
        .body(axum::body::Body::from(output.bytes.to_vec()))
        .map_err(|_| AppError::internal().into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreservedInput {
    account_id: String,
    before: Option<i64>,
    #[serde(default)]
    query: werc_domain::moments::MomentQuery,
}
pub(super) async fn preserved(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<PreservedInput>,
) -> Result<Json<werc_application::moment_preservation::PreservedMomentPage>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    authorize(&state.application.authenticate(&secret)?, &input.account_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let result = local_work(state.application, move |app| {
        app.preserved_moments_local(&input.account_id, input.before, &input.query)
    })
    .await?;
    authorize(&app.authenticate(&secret)?, &account)?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreservedExportInput {
    account_id: String,
    #[serde(default)]
    query: werc_domain::moments::MomentQuery,
    format: werc_application::live_moments::MomentExportFormat,
    request_id: String,
}
pub(super) async fn preserved_export(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<PreservedExportInput>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    authorize(&p, &input.account_id)?;
    p.require("exports:create")?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let account = input.account_id.clone();
    let format = input.format;
    let app = state.application.clone();
    let output = local_work(state.application, move |app| {
        app.export_preserved_moments_local(&input.account_id, &input.query, format, &request)
    })
    .await?;
    let p = app.authenticate(&secret)?;
    authorize(&p, &account)?;
    p.require("exports:create")?;
    Response::builder()
        .header("Content-Type", format.content_type())
        .header(
            "Content-Disposition",
            format!(
                "attachment; filename=\"WeRC-preserved-moments.{}\"",
                format.extension()
            ),
        )
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("X-WeRC-Records", output.summary.rows.to_string())
        .body(axum::body::Body::from(output.bytes.to_vec()))
        .map_err(|_| AppError::internal().into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MediaInput {
    account_id: String,
    view_id: Option<String>,
    selection: werc_domain::moments::MomentMediaSelection,
    request_id: String,
}
pub(super) async fn media(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<MediaInput>,
) -> Result<Json<werc_application::moment_media::MomentMediaManifest>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    authorize(&p, &input.account_id)?;
    p.require("media:read")?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let result = local_work(state.application, move |app| {
        if let Some(m) = app.cached_moment_media_local(&input.account_id, &input.selection)? {
            return Ok(m);
        }
        app.fetch_moment_media_local(
            &input.account_id,
            input
                .view_id
                .as_deref()
                .ok_or_else(|| AppError::new("MOMENT_MEDIA_NOT_SAVED", "此媒体尚未保留"))?,
            &input.selection,
            &request,
        )
    })
    .await?;
    let p = app.authenticate(&secret)?;
    authorize(&p, &account)?;
    p.require("media:read")?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MediaContentInput {
    account_id: String,
    id: String,
}
pub(super) async fn media_content(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<MediaContentInput>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    authorize(&p, &input.account_id)?;
    p.require("media:read")?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let (m, b) = local_work(state.application, move |app| {
        app.moment_media_contents_local(&input.account_id, &input.id)
    })
    .await?;
    let p = app.authenticate(&secret)?;
    authorize(&p, &account)?;
    p.require("media:read")?;
    Ok((
        [
            (header::CONTENT_TYPE, m.mime),
            (header::CACHE_CONTROL, "no-store".into()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            (header::CONTENT_DISPOSITION, "attachment".into()),
        ],
        b.to_vec(),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BundleInput {
    account_id: String,
    view_id: Option<String>,
    query: werc_domain::moments::MomentQuery,
    request_id: String,
}
pub(super) async fn bundle(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<BundleInput>,
) -> Result<Response, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let p = state.application.authenticate(&secret)?;
    authorize(&p, &input.account_id)?;
    p.require("exports:create")?;
    p.require("media:read")?;
    let request = request_key(&p, &input.account_id, &input.request_id)?;
    let app = state.application.clone();
    let account = input.account_id.clone();
    let output = local_work(state.application, move |app| {
        struct Bounded(std::io::Cursor<zeroize::Zeroizing<Vec<u8>>>);
        impl std::io::Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                let end = usize::try_from(self.0.position())
                    .ok()
                    .and_then(|n| n.checked_add(bytes.len()))
                    .filter(|n| *n <= 256 * 1024 * 1024)
                    .ok_or_else(|| std::io::Error::other("bundle size limit"))?;
                let start = self.0.position() as usize;
                let length = self.0.get_ref().len().max(end);
                self.0.get_mut().resize(length, 0);
                self.0.get_mut()[start..end].copy_from_slice(bytes);
                self.0.set_position(end as u64);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl std::io::Seek for Bounded {
            fn seek(&mut self, at: std::io::SeekFrom) -> std::io::Result<u64> {
                self.0.seek(at)
            }
        }
        let mut out = Bounded(std::io::Cursor::new(zeroize::Zeroizing::new(Vec::new())));
        app.export_moment_bundle_local(
            &input.account_id,
            input.view_id.as_deref(),
            &input.query,
            &request,
            &mut out,
        )?;
        Ok(out.0.into_inner())
    })
    .await?;
    let p = app.authenticate(&secret)?;
    authorize(&p, &account)?;
    p.require("exports:create")?;
    p.require("media:read")?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=werc-moments.zip",
            ),
        ],
        output.to_vec(),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActionInput {
    account_id: String,
    action: werc_domain::outbox::MomentAction,
    #[serde(default)]
    text: String,
}
pub(super) async fn interact(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<ActionInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = input.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.create_moment_action(&p, &input.account_id, &input.action, &input.text, &key)
    })
    .await?;
    let p = application.authenticate(&secret)?;
    p.require("moments:publish")?;
    p.require("operations:create")?;
    p.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}
