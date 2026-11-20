use super::*;

pub(super) async fn file_assets(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let account = query.account_id.clone();
    let assets = local_work(state.application.clone(), move |app| {
        app.file_assets(&principal, &query.account_id)
    })
    .await?;
    let current = state.application.authenticate(bearer(&headers)?)?;
    current.require("media:read")?;
    current.require_account(&account)?;
    Ok(Json(json!({"data":assets})))
}
pub(super) async fn upload_file(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    upload_binary(
        State(state),
        axum::Extension(principal),
        headers,
        Query(query),
        multipart,
        BinaryKind::File,
    )
    .await
}
pub(super) async fn upload_video(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    upload_binary(
        State(state),
        axum::Extension(principal),
        headers,
        Query(query),
        multipart,
        BinaryKind::Video,
    )
    .await
}
pub(super) async fn upload_emoticon(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    upload_binary(
        State(state),
        axum::Extension(principal),
        headers,
        Query(query),
        multipart,
        BinaryKind::Emoticon,
    )
    .await
}
enum BinaryKind {
    File,
    Video,
    Emoticon,
}
async fn upload_binary(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    mut multipart: Multipart,
    kind: BinaryKind,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    principal.require("media:write")?;
    principal.require_account(&query.account_id)?;
    let _permit = state
        .uploads
        .try_acquire()
        .map_err(|_| AppError::new("BUSY", "同时最多处理两个文件上传，请稍后重试"))?;
    let limit = if matches!(kind, BinaryKind::Emoticon) {
        werc_domain::emoticon::MAX_EMOTICON_BYTES
    } else {
        werc_domain::file::MAX_FILE_BYTES
    };
    let read = async {
        let mut bytes = Vec::new();
        let mut found = false;
        let mut filename = String::new();
        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|_| AppError::new("INVALID_ARGUMENT", "multipart 上传无效或超过限额"))?
        {
            if found || field.name() != Some("file") {
                return Err(AppError::new(
                    "INVALID_ARGUMENT",
                    "仅接受一个名为 file 的文件字段",
                ));
            }
            filename = field
                .file_name()
                .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "缺少文件名"))?
                .to_owned();
            werc_domain::file::validate_file_name(&filename)?;
            found = true;
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|_| AppError::new("INVALID_ARGUMENT", "文件上传读取失败或超过限额"))?
            {
                if bytes.len() + chunk.len() > limit {
                    return Err(AppError::new(
                        "QUOTA_EXCEEDED",
                        "文件超过当前媒体类型的大小限额",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
        }
        if !found {
            return Err(AppError::new("INVALID_ARGUMENT", "缺少 file 字段"));
        }
        Ok((filename, bytes))
    };
    let (filename, bytes) = tokio::time::timeout(Duration::from_secs(60), read)
        .await
        .map_err(|_| AppError::new("INVALID_ARGUMENT", "上传总时限为 60 秒"))??;
    let current = state.application.authenticate(bearer(&headers)?)?;
    let account = query.account_id.clone();
    let asset = local_work(state.application.clone(), move |app| {
        current.require("media:write")?;
        current.require_account(&query.account_id)?;
        match kind {
            BinaryKind::Video => app.upload_video_local(&query.account_id, &filename, bytes),
            BinaryKind::Emoticon => app.upload_emoticon_local(&query.account_id, &filename, bytes),
            BinaryKind::File => app.upload_file(&current, &query.account_id, &filename, bytes),
        }
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
pub(super) async fn download_file(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let account = query.account_id.clone();
    let bytes = local_work(state.application.clone(), move |app| {
        app.download_file(&principal, &query.account_id, &id)
    })
    .await?;
    let current = state.application.authenticate(bearer(&headers)?)?;
    current.require("media:read")?;
    current.require_account(&account)?;
    let encoded = bytes
        .file_name
        .as_bytes()
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    let disposition = format!("attachment; filename*=UTF-8''{encoded}");
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
        ],
        bytes.bytes.to_vec(),
    )
        .into_response())
}
pub(super) async fn delete_file(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    local_work(state.application, move |app| {
        app.delete_file(&principal, &query.account_id, &id)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SendFile {
    account_id: String,
    username: String,
    asset_id: String,
}
pub(super) async fn send_file(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<SendFile>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok());
    if !key
        .is_some_and(|s| (16..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        || input.username.trim().is_empty()
        || input.username.len() > 256
        || Uuid::parse_str(&input.asset_id).is_err()
    {
        return Err(
            AppError::new("INVALID_ARGUMENT", "需要有效的账号、目标、文件资产和幂等键").into(),
        );
    }
    state.application.require_wechat_capability(
        &principal,
        &input.account_id,
        "messages:send_file",
        "messages.send_file",
    )?;
    let key = key.ok_or_else(AppError::internal)?.to_owned();
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = input.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.create_file_send(
            &principal,
            &input.account_id,
            &input.username,
            &input.asset_id,
            &key,
        )
    })
    .await?;
    let principal = application.authenticate(&secret)?;
    principal.require("messages:send_file")?;
    principal.require("operations:create")?;
    principal.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}

pub(super) async fn send_video(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<SendFile>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok());
    if !key
        .is_some_and(|s| (16..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        || input.username.trim().is_empty()
        || input.username.len() > 256
        || Uuid::parse_str(&input.asset_id).is_err()
    {
        return Err(
            AppError::new("INVALID_ARGUMENT", "需要有效的账号、目标、视频资产和幂等键").into(),
        );
    }
    state.application.require_wechat_capability(
        &principal,
        &input.account_id,
        "messages:send_video",
        "messages.send_video",
    )?;
    let key = key.ok_or_else(AppError::internal)?.to_owned();
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = input.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.create_video_send(
            &principal,
            &input.account_id,
            &input.username,
            &input.asset_id,
            &key,
        )
    })
    .await?;
    let principal = application.authenticate(&secret)?;
    principal.require("messages:send_video")?;
    principal.require("operations:create")?;
    principal.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}

pub(super) async fn download_video(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let account = query.account_id.clone();
    let bytes = local_work(state.application.clone(), move |app| {
        let content = app.download_file(&principal, &query.account_id, &id)?;
        werc_domain::video::inspect_send_mp4(&content.file_name, &content.bytes)?;
        Ok(content)
    })
    .await?;
    let current = state.application.authenticate(bearer(&headers)?)?;
    current.require("media:read")?;
    current.require_account(&account)?;
    Ok((
        [
            (header::CONTENT_TYPE, "video/mp4"),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::ACCEPT_RANGES, "none"),
        ],
        bytes.bytes.to_vec(),
    )
        .into_response())
}

pub(super) async fn send_emoticon(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<SendFile>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok());
    if !key
        .is_some_and(|s| (16..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        || input.username.trim().is_empty()
        || input.username.len() > 256
        || Uuid::parse_str(&input.asset_id).is_err()
    {
        return Err(
            AppError::new("INVALID_ARGUMENT", "需要有效的账号、目标、表情资产和幂等键").into(),
        );
    }
    state.application.require_wechat_capability(
        &principal,
        &input.account_id,
        "messages:send_emoticon",
        "messages.send_emoticon",
    )?;
    let key = key.ok_or_else(AppError::internal)?.to_owned();
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = input.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.create_emoticon_send(
            &principal,
            &input.account_id,
            &input.username,
            &input.asset_id,
            &key,
        )
    })
    .await?;
    let principal = application.authenticate(&secret)?;
    principal.require("messages:send_emoticon")?;
    principal.require("operations:create")?;
    principal.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}

pub(super) async fn download_emoticon(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let account = query.account_id.clone();
    let bytes = local_work(state.application.clone(), move |app| {
        principal.require("media:read")?;
        principal.require_account(&query.account_id)?;
        app.emoticon_contents_local(&query.account_id, &id)
    })
    .await?;
    let current = state.application.authenticate(bearer(&headers)?)?;
    current.require("media:read")?;
    current.require_account(&account)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                if bytes.file_name.ends_with(".png") {
                    "image/png"
                } else {
                    "image/gif"
                },
            ),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::ACCEPT_RANGES, "none"),
        ],
        bytes.bytes.to_vec(),
    )
        .into_response())
}
