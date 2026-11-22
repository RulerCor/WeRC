mod backfill;
mod wechat_settings;
mod bot;
mod bot_send;
mod browse;
mod contacts;
mod conversation;
mod conversation_media;
mod directory_export;
mod emoticon;
mod favorites;
mod files;
mod groups;
mod images;
mod library;
mod articles;
mod live_calendar;
mod live_export;
mod live_search;
mod mcp;
mod moments;
mod outbox;
mod receive;
mod message_preservation;
mod send_text;
mod sessions;
mod special_records;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;
use werc_application::Application;
use werc_domain::{AppError, Principal};

#[derive(Clone)]
pub struct ApiState {
    pub application: Arc<Application>,
    uploads: Arc<tokio::sync::Semaphore>,
    history: Arc<tokio::sync::Semaphore>,
    send_waiters: Arc<tokio::sync::Semaphore>,
}

struct ApiError(AppError);
impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code.as_str() {
            "UNAUTHORIZED" | "AUTH_REVOKED" => StatusCode::UNAUTHORIZED,
            "INVALID_TARGET" | "MENTION_SELECTION_INVALID" | "QUOTE_SOURCE_INVALID" => {
                StatusCode::BAD_REQUEST
            }
            "IDENTITY_UNVERIFIED"
            | "CONNECTION_CHANGED"
            | "APPROVAL_EXPIRED"
            | "APPROVAL_MISMATCH"
            | "CURRENT_CONNECTION_REQUIRED" => StatusCode::CONFLICT,
            "NOT_FOUND" | "OUTBOX_NOT_FOUND" | "RECEIVED_MEDIA_NOT_FOUND" => StatusCode::NOT_FOUND,
            "FORBIDDEN" | "ORIGIN_REJECTED" => StatusCode::FORBIDDEN,
            "API_PAUSED" | "NATIVE_DISCONNECTED" | "NATIVE_NOT_CONNECTED" | "NATIVE_COMPONENT_UNAVAILABLE" => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            "WECHAT_SETTINGS_UNAVAILABLE" | "CAPABILITY_NOT_IMPLEMENTED" | "LIVE_QUERY_UNAVAILABLE" | "SCHEDULING_UNAVAILABLE" => {
                StatusCode::NOT_IMPLEMENTED
            }
            "INVALID_ARGUMENT"
            | "INVALID_EXECUTABLE"
            | "LIVE_QUERY_INVALID"
            | "LIVE_CALENDAR_INVALID"
            | "SCHEDULE_INVALID" => StatusCode::BAD_REQUEST,
            "ACCOUNT_NOT_FOUND"
            | "ASSET_NOT_FOUND"
            | "OPERATION_NOT_FOUND"
            | "SOURCE_NOT_FOUND" => StatusCode::NOT_FOUND,
            "EMOTICON_LAYOUT_UNSUPPORTED"
            | "RECEIVED_EMOTICON_INVALID"
            | "FORWARD_MEDIA_INVALID"
            | "UNSUPPORTED_EMOTICON"
            | "UNSUPPORTED_MEDIA"
            | "UNSUPPORTED_IMAGE"
            | "UNSUPPORTED_RECEIVED_IMAGE"
            | "UNSUPPORTED_VIDEO" => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "LIVE_EXPORT_LIMIT"
            | "DIRECTORY_EXPORT_LIMIT"
            | "DIRECTORY_EXPORT_CELL_LIMIT"
            | "QUOTA_EXCEEDED"
            | "FORWARD_FILE_CACHE_LIMIT" => StatusCode::PAYLOAD_TOO_LARGE,
            "IDEMPOTENCY_CONFLICT"
            | "INVALID_TRANSITION"
            | "CURSOR_STALE"
            | "SOURCE_CONFLICT"
            | "LIVE_CONTACT_ANCHOR_CHANGED"
            | "LIVE_SESSION_ANCHOR_CHANGED"
            | "LIVE_SOURCE_REPLACED" => StatusCode::CONFLICT,
            "QUERY_CANCELED" => StatusCode::CONFLICT,
            "QUERY_TIMEOUT" | "LIVE_READ_TIMEOUT" | "LIVE_WORKER_TIMEOUT" => {
                StatusCode::REQUEST_TIMEOUT
            }
            "EMOTICON_CACHE_UNAVAILABLE"
            | "FORWARD_FILE_CACHE_UNAVAILABLE"
            | "MESSAGE_NOT_FOUND" => StatusCode::NOT_FOUND,
            "BUSY" | "RECEIVED_MEDIA_BUSY" | "NATIVE_TARGET_LIMIT" => StatusCode::TOO_MANY_REQUESTS,
            "RECEIVED_MEDIA_INVALID" => StatusCode::UNPROCESSABLE_ENTITY,
            "RECOVERY_SOURCES_PENDING" => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(json!({"error": self.0, "request_id": Uuid::new_v4()})),
        )
            .into_response()
    }
}

pub fn router(application: Arc<Application>) -> Router {
    let state = ApiState {
        application,
        uploads: Arc::new(tokio::sync::Semaphore::new(2)),
        history: Arc::new(tokio::sync::Semaphore::new(2)),
        send_waiters: Arc::new(tokio::sync::Semaphore::new(16)),
    };
    let api = api_routes(state.clone());
    let mcp_routes = Router::new()
        .route(
            "/mcp",
            post(mcp::endpoint).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route(
            "/mcp/read",
            post(mcp::read_endpoint).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    Router::new()
        .merge(mcp_routes)
        .route(
            "/health",
            get(|| async {
                Json(json!({"product":"WeRC_Mon3trBot", "version":env!("CARGO_PKG_VERSION")}))
            }),
        )
        .nest("/api/v1", api)
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(
                    json!({"error":{"code":"NOT_FOUND","message":"接口不存在","retryable":false}}),
                ),
            )
        })
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

fn api_routes(state: ApiState) -> Router<ApiState> {
    Router::new()
        .route("/wechat/settings/capabilities", get(wechat_settings::capabilities))
        .route("/wechat/settings/read", post(wechat_settings::read))
        .route("/wechat/settings/plan", post(wechat_settings::plan))
        .route("/wechat/settings/apply", post(wechat_settings::apply))
        .route("/wechat/settings/changes", get(wechat_settings::changes))
        .route("/wechat/settings/changes/{id}/restore", post(wechat_settings::restore))
        .route("/messages/live/emoticon", post(emoticon::live_save))
        .route("/messages/emoticon/cancel", post(emoticon::cancel))
        .route(
            "/messages/native/{position}/emoticon",
            get(emoticon::content)
                .post(emoticon::receive)
                .delete(emoticon::delete),
        )
        .route("/accounts", get(accounts))
        .route("/connections", get(connection))
        .route("/capabilities", get(capabilities))
        .route("/openapi.json", get(openapi))
        .route("/events", get(events))
        .route("/events/stream", get(event_stream))
        .route("/messages", get(messages))
        .route("/messages/native", get(receive::list))
        .route("/messages/native/stream", get(bot::message_stream))
        .route("/recipients/prepare", post(bot::prepare_recipient))
        .route("/outbox/{outbox_id}/submit", post(bot::submit))
        .route("/outbox/{outbox_id}/reconcile", post(bot::reconcile))
        .route("/messages/send/mention", post(bot_send::mention))
        .route("/messages/send/quote", post(bot_send::quote))
        .route(
            "/messages/native/{position}/video",
            get(receive::video_content)
                .post(receive::video_download)
                .delete(receive::video_delete),
        )
        .route(
            "/messages/native/{position}/voice",
            get(receive::voice_content),
        )
        .route(
            "/messages/native/{position}/image",
            get(receive::image_content).post(receive::image_download),
        )
        .route("/browse/conversations", post(browse::conversations))
        .route("/browse/messages", post(browse::messages))
        .route("/browse/cancel", post(browse::cancel))
        .route("/sources", get(history_sources))
        .route("/contacts", get(history_contacts))
        .route("/contacts/live", post(contacts::page))
        .route("/contacts/group-names", get(contacts::group_names))
        .route("/contacts/friends", get(contacts::friends))
        .route("/contacts/directory", get(contacts::directory))
        .route("/contacts/live/avatar", post(contacts::avatar))
        .route("/contacts/live/group", post(groups::page))
        .route("/contacts/live/group/cancel", post(groups::cancel))
        .route("/contacts/live/cancel", post(contacts::cancel))
        .route("/contacts/live/export", post(directory_export::create))
        .route(
            "/contacts/live/export/cancel",
            post(directory_export::cancel),
        )
        .route(
            "/messages/live/conversation/media",
            post(conversation_media::save).delete(conversation_media::delete),
        )
        .route(
            "/messages/live/conversation/media/content",
            post(conversation_media::content),
        )
        .route(
            "/messages/live/conversation/media/cancel",
            post(conversation_media::cancel),
        )
        .route("/messages/live/export", post(live_export::create))
        .route("/messages/live/export/cancel", post(live_export::cancel))
        .route("/messages/live/calendar", post(live_calendar::page))
        .route("/messages/live/calendar/cancel", post(conversation::cancel))
        .route("/messages/live/search", post(live_search::page))
        .route("/messages/recovered", get(backfill::page))
        .route("/messages/recovered/status", get(backfill::status))
        .route("/messages/recovered/stream", get(backfill::stream))
        .route("/messages/recovery", post(library::capture))
        .route("/messages/recovery/export", post(library::capture_export))
        .route(
            "/messages/recovery/media/content",
            post(library::capture_media_content),
        )
        .route("/library/records", post(library::records))
        .route("/articles", post(articles::action))
        .route("/articles/export", post(articles::export))
        .route("/articles/cancel", post(articles::cancel))
        .route("/articles/jobs", post(articles::jobs))
        .route(
            "/messages/recovery/records/export",
            post(library::record_export),
        )
        .route("/audio/asr-status", get(library::asr_status))
        .route("/audio/transcribe", post(library::transcribe))
        .route("/messages/live/search/cancel", post(conversation::cancel))
        .route("/messages/live/conversation", post(conversation::page))
        .route(
            "/messages/live/conversation/multi",
            post(conversation::multi_page),
        )
        .route(
            "/messages/live/conversation/cancel",
            post(conversation::cancel),
        )
        .route("/records/live", post(special_records::page))
        .route("/records/live/export", post(special_records::export))
        .route("/records/live/cancel", post(special_records::cancel))
        .route("/favorites/live", post(favorites::page))
        .route("/favorites/live/media", post(favorites::content))
        .route("/favorites/live/copy", post(favorites::copy))
        .route("/favorites/live/cancel", post(favorites::cancel))
        .route("/moments/preserved", post(moments::preserved))
        .route("/moments/preserved/export", post(moments::preserved_export))
        .route("/moments/bundle", post(moments::bundle))
        .route("/moments/media", post(moments::media))
        .route("/moments/media/content", post(moments::media_content))
        .route("/moments/live", post(moments::page))
        .route("/moments/live/overview", post(moments::overview))
        .route("/moments/live/export", post(moments::export))
        .route("/moments/live/cancel", post(moments::cancel))
        .route("/sessions/live", post(sessions::page))
        .route("/sessions/live/cancel", post(sessions::cancel))
        .route("/sessions", get(history_sessions))
        .route("/exports/messages", get(export_history))
        .route("/outbox", get(outbox::list))
        .route("/outbox/history", get(outbox::history))
        .route("/outbox/{outbox_id}/local-record", get(outbox::local_record))
        .route(
            "/outbox/{outbox_id}",
            get(outbox::status).delete(outbox::cancel),
        )
        .route("/messages/send-text", post(send_text::send_text))
        .route("/messages/send-voice", post(send_voice))
        .route("/messages/send-image", post(images::send_image))
        .route(
            "/messages/native/{position}/file",
            get(receive::file_content).post(receive::file_download),
        )
        .route("/messages/send-file", post(files::send_file))
        .route("/messages/send-video", post(files::send_video))
        .route(
            "/media/videos",
            post(files::upload_video).layer(DefaultBodyLimit::max(
                werc_domain::file::MAX_FILE_BYTES + 65536,
            )),
        )
        .route(
            "/media/videos/{asset_id}",
            get(files::download_video).delete(files::delete_file),
        )
        .route("/messages/send-emoticon", post(files::send_emoticon))
        .route(
            "/media/emoticons",
            post(files::upload_emoticon).layer(DefaultBodyLimit::max(
                werc_domain::emoticon::MAX_EMOTICON_BYTES + 65536,
            )),
        )
        .route(
            "/media/emoticons/{asset_id}",
            get(files::download_emoticon).delete(files::delete_file),
        )
        .route(
            "/media/files",
            get(files::file_assets)
                .post(files::upload_file)
                .layer(DefaultBodyLimit::max(
                    werc_domain::file::MAX_FILE_BYTES + 65536,
                )),
        )
        .route(
            "/media/files/{asset_id}",
            get(files::download_file).delete(files::delete_file),
        )
        .route(
            "/media/images",
            get(images::image_assets)
                .post(images::upload_image)
                .layer(DefaultBodyLimit::max(
                    werc_domain::image::MAX_IMAGE_BYTES + 65536,
                )),
        )
        .route(
            "/media/images/{asset_id}",
            get(images::download_image).delete(images::delete_image),
        )
        .route(
            "/media/audio",
            get(audio_assets)
                .post(upload_audio)
                .layer(DefaultBodyLimit::max(
                    werc_domain::media::MAX_AUDIO_BYTES + 65536,
                )),
        )
        .route(
            "/media/audio/{asset_id}",
            get(download_audio).delete(delete_audio),
        )
        .route("/operations", get(operations).post(create_operation))
        .route(
            "/operations/{operation_id}",
            get(operation).delete(cancel_operation),
        )
        .route("/messages/preservation", post(message_preservation::handle))
        .route("/moments/publish", post(publish))
        .route("/moments/interact", post(moments::interact))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate))
}

fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| {
            ApiError(AppError::new(
                "UNAUTHORIZED",
                "需要 Bearer 令牌；不接受 URL 查询令牌",
            ))
        })
}

async fn authenticate(State(state): State<ApiState>, mut request: Request, next: Next) -> Response {
    if request.headers().contains_key(header::ORIGIN) {
        return ApiError(AppError::new(
            "ORIGIN_REJECTED",
            "此接口仅面向非浏览器 API 客户端；浏览器 Origin 未授权",
        ))
        .into_response();
    }
    let principal = bearer(request.headers())
        .and_then(|token| state.application.authenticate(token).map_err(Into::into));
    match principal {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            let mut response = next.run(request).await;
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                "no-store".parse().expect("静态头有效"),
            );
            response.headers_mut().insert(
                "x-content-type-options",
                "nosniff".parse().expect("静态头有效"),
            );
            response
        }
        Err(error) => error.into_response(),
    }
}

async fn accounts(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        json!({"data":state.application.accounts(&principal)?}),
    ))
}

async fn connection(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        json!({"data":state.application.binding(&principal, &query.account_id)?, "current_connection":state.application.current_connection_local()?}),
    ))
}

async fn capabilities(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<serde_json::Value>, ApiError> {
    principal.require("capabilities:read")?;
    Ok(Json(json!({"data":state.application.capabilities()})))
}

async fn openapi(
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<serde_json::Value>, ApiError> {
    principal.require("openapi:read")?;
    Ok(Json(
        serde_json::from_str(include_str!("../../../contracts/openapi.json"))
            .map_err(|_| AppError::internal())?,
    ))
}

#[derive(Deserialize)]
struct EventQuery {
    account_id: String,
    #[serde(default)]
    after: i64,
    #[serde(default = "default_limit")]
    limit: u32,
}
fn default_limit() -> u32 {
    100
}
async fn events(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<EventQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = state
        .application
        .events(&principal, &query.account_id, query.after, query.limit)?;
    let next = data.last().map_or(query.after, |event| event.event_id);
    Ok(Json(json!({"data":data,"next_cursor":next})))
}

async fn event_stream(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<EventQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let token = bearer(&headers)?.to_owned();
    let principal = state.application.authenticate(&token)?;
    let mut cursor = if let Some(last) = headers.get("last-event-id") {
        last.to_str()
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "Last-Event-ID 必须是非负整数"))?
    } else {
        query.after
    };
    state
        .application
        .events(&principal, &query.account_id, cursor, query.limit)?;
    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let result = state.application.authenticate(&token).and_then(|p| state.application.events(&p, &query.account_id, cursor, query.limit));
            match result {
                Ok(events) => for event in events {
                    if let Err(error) = state.application.authenticate(&token).and_then(|p| {
                        p.require("events:read")?;
                        p.require_account(&query.account_id)
                    }) {
                        yield Ok(SseEvent::default().event("stream.closed").data(json!({"code":error.code}).to_string()));
                        return;
                    }
                    cursor = event.event_id;
                    if let Ok(serialized) = serde_json::to_string(&event) {
                        yield Ok(SseEvent::default().id(cursor.to_string()).event(&event.event_type).data(serialized));
                    }
                },
                Err(error) => {
                    yield Ok(SseEvent::default().event("stream.closed").data(json!({"code":error.code}).to_string()));
                    break;
                }
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    ))
}

#[derive(Deserialize)]
struct AccountQuery {
    account_id: String,
}
#[derive(Deserialize)]
struct HistoryParams {
    account_id: String,
    conversation_id: Option<String>,
    sender_id: Option<String>,
    message_type: Option<i64>,
    search: Option<String>,
    from: Option<i64>,
    until: Option<i64>,
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    format: Option<String>,
}
impl HistoryParams {
    fn query(&self) -> werc_domain::history::HistoryQuery {
        werc_domain::history::HistoryQuery {
            conversation_id: self.conversation_id.clone(),
            sender_id: self.sender_id.clone(),
            message_type: self.message_type,
            search: self.search.clone(),
            from: self.from,
            until: self.until,
            cursor: self.cursor.clone(),
            limit: self.limit,
        }
    }
}
async fn messages(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<HistoryParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let page = history_work(state, move |app| {
        app.history_messages(&principal, &query.account_id, &query.query())
    })
    .await?;
    Ok(Json(
        serde_json::to_value(page).map_err(|_| AppError::internal())?,
    ))
}
async fn history_sources(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(q): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = history_work(state, move |app| app.history_sources(&p, &q.account_id)).await?;
    Ok(Json(json!({"data":data,"data_mode":"imported_snapshot"})))
}
async fn history_contacts(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(q): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = history_work(state, move |app| app.history_contacts(&p, &q.account_id)).await?;
    Ok(Json(json!({"data":data,"data_mode":"imported_snapshot"})))
}
async fn history_sessions(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(q): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let data = history_work(state, move |app| app.history_sessions(&p, &q.account_id)).await?;
    Ok(Json(json!({"data":data,"data_mode":"imported_snapshot"})))
}
async fn export_history(
    State(state): State<ApiState>,
    axum::Extension(p): axum::Extension<Principal>,
    Query(q): Query<HistoryParams>,
) -> Result<Response, ApiError> {
    let format = q.format.clone().unwrap_or_else(|| "json".into());
    let (mime, name) = match format.as_str() {
        "json" => ("application/json; charset=utf-8", "messages.json"),
        "csv" => ("text/csv; charset=utf-8", "messages.csv"),
        "txt" => ("text/plain; charset=utf-8", "messages.txt"),
        "jsonl" => ("application/x-ndjson; charset=utf-8", "messages.jsonl"),
        "html" => ("text/html; charset=utf-8", "messages.html"),
        "md" => ("text/markdown; charset=utf-8", "messages.md"),
        "sql" => ("application/sql; charset=utf-8", "messages.sql"),
        _ => return Err(AppError::new("INVALID_ARGUMENT", "导出格式未支持").into()),
    };
    let output = history_work(state, move |app| {
        app.export_history(&p, &q.account_id, &q.query(), &format)
    })
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (
                header::CONTENT_DISPOSITION,
                &format!("attachment; filename=\"{name}\""),
            ),
        ],
        output,
    )
        .into_response())
}

async fn history_work<T: Send + 'static>(
    state: ApiState,
    action: impl FnOnce(&Application) -> Result<T, AppError> + Send + 'static,
) -> Result<T, ApiError> {
    let permit = state
        .history
        .try_acquire_owned()
        .map_err(|_| AppError::new("BUSY", "同时最多执行两个历史查询或导出，请稍后重试"))?;
    local_work(state.application, move |app| {
        // HTTP 请求取消后，阻塞工作仍持有许可，避免解密任务突破并发上限。
        let _permit = permit;
        action(app)
    })
    .await
}

#[cfg(test)]
mod history_admission_tests {
    use super::*;
    use werc_application::ports::CapabilityProvider;
    struct Provider;
    impl CapabilityProvider for Provider {
        fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
            Vec::new()
        }
    }
    #[tokio::test]
    async fn cancelled_request_does_not_release_a_running_decryption_permit() {
        let state = ApiState {
            application: Arc::new(Application::new(
                Arc::new(werc_infrastructure::SqliteRepository::in_memory().unwrap()),
                Arc::new(Provider),
            )),
            uploads: Arc::new(tokio::sync::Semaphore::new(2)),
            history: Arc::new(tokio::sync::Semaphore::new(2)),
            send_waiters: Arc::new(tokio::sync::Semaphore::new(16)),
        };
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let copy = state.clone();
        let request = tokio::spawn(async move {
            history_work(copy, move |_| {
                let _ = started_tx.send(());
                let _ = finish_rx.recv();
                Ok(())
            })
            .await
            .map_err(|e| e.0.code)
        });
        started_rx.await.unwrap();
        request.abort();
        let _ = request.await;
        let available = state.history.available_permits();
        let other = state.history.clone().try_acquire_owned().unwrap();
        let rejected = history_work(state.clone(), |_| Ok(())).await;
        let _ = finish_tx.send(());
        drop(other);
        let all = tokio::time::timeout(Duration::from_secs(3), state.history.acquire_many(2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(available, 1);
        assert_eq!(rejected.err().unwrap().0.code, "BUSY");
        drop(all);
    }
}

async fn local_work<T: Send + 'static>(
    app: Arc<Application>,
    action: impl FnOnce(&Application) -> Result<T, AppError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(move || action(&app))
        .await
        .map_err(|_| AppError::internal())?
        .map_err(Into::into)
}

async fn audio_assets(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let assets = local_work(state.application, move |app| {
        app.audio_assets(&principal, &query.account_id)
    })
    .await?;
    Ok(Json(json!({"data":assets})))
}
async fn upload_audio(
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
        .map_err(|_| AppError::new("BUSY", "同时最多处理两个音频上传，请稍后重试"))?;
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
                    "仅接受一个名为 file 的音频文件字段",
                ));
            }
            found = true;
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|_| AppError::new("INVALID_ARGUMENT", "音频上传读取失败或超过限额"))?
            {
                if bytes.len() + chunk.len() > werc_domain::media::MAX_AUDIO_BYTES {
                    return Err(AppError::new("QUOTA_EXCEEDED", "单个音频最多 8 MiB"));
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
    let asset = local_work(state.application.clone(), move |app| {
        app.upload_audio(&current, &query.account_id, bytes)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data":asset,"sent":false})),
    ))
}
async fn download_audio(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let bytes = local_work(state.application, move |app| {
        app.download_audio(&principal, &query.account_id, &id)
    })
    .await?;
    Ok((
        [
            (header::CONTENT_TYPE, "audio/wav"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=audio.wav",
            ),
        ],
        bytes.to_vec(),
    )
        .into_response())
}
async fn delete_audio(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    local_work(state.application, move |app| {
        app.delete_audio(&principal, &query.account_id, &id)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendVoice {
    account_id: String,
    username: String,
    asset_id: String,
}
async fn send_voice(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<SendVoice>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok());
    if !key
        .is_some_and(|s| (16..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        || input.username.trim().is_empty()
        || input.username.len() > 256
        || Uuid::parse_str(&input.asset_id).is_err()
    {
        return Err(
            AppError::new("INVALID_ARGUMENT", "需要有效的账号、目标、音频资产和幂等键").into(),
        );
    }
    state.application.require_wechat_capability(
        &principal,
        &input.account_id,
        "messages:send_voice",
        "messages.send_voice",
    )?;
    let key = key.ok_or_else(AppError::internal)?.to_owned();
    let secret = bearer(&headers)?.to_owned();
    let application = state.application.clone();
    let account = input.account_id.clone();
    let data = local_work(state.application, move |app| {
        app.create_voice_send(
            &principal,
            &input.account_id,
            &input.username,
            &input.asset_id,
            &key,
        )
    })
    .await?;
    let principal = application.authenticate(&secret)?;
    principal.require("messages:send_voice")?;
    principal.require("operations:create")?;
    principal.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectAudioTask {
    account_id: String,
    asset_id: String,
}
async fn create_operation(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<InspectAudioTask>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let op = local_work(state.application, move |app| {
        app.inspect_audio_task(&principal, &input.account_id, &input.asset_id, &key)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({"data":op,"sent":false}))))
}
async fn operations(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        json!({"data":local_work(state.application, move |app| app.operations(&principal, &query.account_id)).await?}),
    ))
}
async fn operation(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        json!({"data":local_work(state.application, move |app| app.operation(&principal, &query.account_id, &id)).await?}),
    ))
}
async fn cancel_operation(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Query(query): Query<AccountQuery>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    local_work(state.application, move |app| {
        app.cancel_operation(&principal, &query.account_id, &id)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishMoment {
    account_id: String,
    video_asset_id: Option<String>,
    #[serde(default)]
    image_asset_ids: Vec<String>,
    text: String,
    visibility: werc_domain::outbox::MomentVisibility,
}
async fn publish(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<PublishMoment>,
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
        if let Some(video) = &input.video_asset_id {
            if input.image_asset_ids.len() != 1 {
                return Err(AppError::new("INVALID_ARGUMENT", "视频动态需要一张封面"));
            }
            return app.create_video_moment(&principal, &input.account_id, &input.text,
                video, &input.image_asset_ids[0], input.visibility, &key);
        }
        app.create_moment(
            &principal,
            &input.account_id,
            &input.text,
            &input.image_asset_ids,
            input.visibility,
            &key,
        )
    })
    .await?;
    let principal = application.authenticate(&secret)?;
    principal.require("moments:publish")?;
    principal.require("operations:create")?;
    principal.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    application: Arc<Application>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "当前版本仅允许回环监听",
        ));
    }
    axum::serve(listener, router(application))
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await
}
