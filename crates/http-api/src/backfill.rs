use super::*;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use std::{convert::Infallible, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryInput {
    account_id: String,
    #[serde(default)]
    after: u64,
    #[serde(default = "one")]
    limit: u32,
}
fn one() -> u32 {
    1
}
fn access(app: &Application, secret: &str, account: &str) -> Result<(), AppError> {
    let p = app.authenticate(secret)?;
    p.require("messages:read")?;
    p.require_account(account)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatusInput {
    account_id: String,
}

pub(super) async fn status(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<StatusInput>,
) -> Result<Json<werc_application::backfill::BackfillStatus>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    access(&state.application, &secret, &query.account_id)?;
    let app = state.application.clone();
    let account = query.account_id.clone();
    let result = local_work(state.application, move |app| {
        app.backfill_status_local(&query.account_id)
    }).await?;
    access(&app, &secret, &account)?;
    Ok(Json(result))
}

pub(super) async fn page(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<QueryInput>,
) -> Result<Json<werc_application::backfill::BackfillPage>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    access(&state.application, &secret, &query.account_id)?;
    let app = state.application.clone();
    let account = query.account_id.clone();
    let output = local_work(state.application, move |app| {
        app.backfill_page_local(&query.account_id, query.after, query.limit)
    })
    .await?;
    access(&app, &secret, &account)?;
    Ok(Json(output))
}
pub(super) async fn stream(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<QueryInput>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    access(&state.application, &secret, &query.account_id)?;
    let mut cursor = match headers.get("last-event-id") {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| {
                AppError::new("INVALID_ARGUMENT", "Last-Event-ID 必须是非负补读页位置")
            })?,
        None => query.after,
    };
    let stream = async_stream::stream! {
        let mut prior_status = None;
        loop {
            let app = state.application.clone();
            let token = secret.clone();
            let account = query.account_id.clone();
            let result = tokio::task::spawn_blocking(move || {
                access(&app, &token, &account)?;
                let page = app.backfill_page_local(&account, cursor, 1)?;
                access(&app, &token, &account)?;
                Ok::<_, AppError>(page)
            }).await.unwrap_or_else(|_| Err(AppError::internal()));
            match result {
                Ok(page) => {
                    let current = serde_json::to_string(&page.status).unwrap_or_default();
                    if prior_status.as_ref() != Some(&current) {
                        yield Ok(SseEvent::default().event("recovery.status").data(current.clone()));
                        prior_status = Some(current);
                    }
                    for batch in page.data {
                        if let Err(error) = access(&state.application, &secret, &query.account_id) {
                            yield Ok(SseEvent::default().event("stream.closed").data(json!({"code":error.code}).to_string()));
                            return;
                        }
                        match serde_json::to_string(&batch) {
                            Ok(body) => {
                                cursor = batch.position;
                                yield Ok(SseEvent::default().id(cursor.to_string()).event("messages.recovered").data(body));
                            }
                            Err(_) => {
                                yield Ok(SseEvent::default().event("stream.closed").data("{\"code\":\"SERIALIZATION_FAILED\"}"));
                                return;
                            }
                        }
                    }
                    if !page.has_more { tokio::time::sleep(Duration::from_secs(1)).await; }
                }
                Err(error) => {
                    yield Ok(SseEvent::default().event("stream.closed").data(json!({"code":error.code}).to_string()));
                    return;
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
