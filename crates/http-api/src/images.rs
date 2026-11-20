use super::*;

pub(super) async fn image_assets(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let account = query.account_id.clone();
    let assets = local_work(state.application.clone(), move |app| {
        app.image_assets(&principal, &query.account_id)
    })
    .await?;
    let current = state.application.authenticate(bearer(&headers)?)?;
    current.require("media:read")?;
    current.require_account(&account)?;
    Ok(Json(json!({"data":assets})))
}
pub(super) async fn upload_image(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    principal.require("media:write")?;
    principal.require_account(&query.account_id)?;
    let _permit = state
        .uploads
        .try_acquire()
        .map_err(|_| AppError::new("BUSY", "同时最多处理两个图片上传，请稍后重试"))?;
    let read = async {
        let mut bytes = Vec::new();
        let mut found = false;
        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| AppError::new("INVALID_ARGUMENT", "multipart 上传无效或超过限额"))?
        {
            if found || field.name() != Some("file") {
                return Err(AppError::new(
                    "INVALID_ARGUMENT",
                    "仅接受一个名为 file 的图片文件字段",
                ));
            }
            found = true;
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|_| AppError::new("INVALID_ARGUMENT", "图片上传读取失败或超过限额"))?
            {
                if bytes.len() + chunk.len() > werc_domain::image::MAX_IMAGE_BYTES {
                    return Err(AppError::new("QUOTA_EXCEEDED", "单个图片最多 8 MiB"));
                }
                bytes.extend_from_slice(&chunk);
            }
        }
        if !found {
            return Err(AppError::new("INVALID_ARGUMENT", "缺少 file 字段"));
        }
        Ok(bytes)
    };
    let bytes = tokio::time::timeout(Duration::from_secs(60), read)
        .await
        .map_err(|_| AppError::new("INVALID_ARGUMENT", "上传总时限为 60 秒"))??;
    let current = state.application.authenticate(bearer(&headers)?)?;
    let account = query.account_id.clone();
    let asset = local_work(state.application.clone(), move |app| {
        app.upload_image(&current, &query.account_id, bytes)
    })
    .await?;
    let current = state.application.authenticate(bearer(&headers)?)?;
    current.require("media:write")?;
    current.require_account(&account)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data":asset,"sent":false})),
    ))
}
pub(super) async fn download_image(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let account = query.account_id.clone();
    let bytes = local_work(state.application.clone(), move |app| {
        app.download_image(&principal, &query.account_id, &id)
    })
    .await?;
    let current = state.application.authenticate(bearer(&headers)?)?;
    current.require("media:read")?;
    current.require_account(&account)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=image.png",
            ),
        ],
        bytes.to_vec(),
    )
        .into_response())
}
pub(super) async fn delete_image(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    local_work(state.application, move |app| {
        app.delete_image(&principal, &query.account_id, &id)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SendImage {
    account_id: String,
    username: String,
    asset_id: String,
}
pub(super) async fn send_image(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<SendImage>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok());
    if !key
        .is_some_and(|s| (16..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        || input.username.trim().is_empty()
        || input.username.len() > 256
        || Uuid::parse_str(&input.asset_id).is_err()
    {
        return Err(
            AppError::new("INVALID_ARGUMENT", "需要有效的账号、目标、图片资产和幂等键").into(),
        );
    }
    state.application.require_wechat_capability(
        &principal,
        &input.account_id,
        "messages:send_image",
        "messages.send_image",
    )?;
    let key = key.ok_or_else(AppError::internal)?.to_owned();
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = input.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.create_image_send(
            &principal,
            &input.account_id,
            &input.username,
            &input.asset_id,
            &key,
        )
    })
    .await?;
    let principal = application.authenticate(&secret)?;
    principal.require("messages:send_image")?;
    principal.require("operations:create")?;
    principal.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}
