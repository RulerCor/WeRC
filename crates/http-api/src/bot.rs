//! Client execution and unfiltered, durable native-message delivery.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Account {
    account_id: String,
}

pub(super) async fn submit(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Account>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let account = input.account_id;
    let application = state.application.clone();
    let auth = secret.clone();
    let result = local_work(application, move |app| {
        app.submit_client_send(&app.authenticate(&auth)?, &account, &id)
    })
    .await?;
    let fresh = state.application.authenticate(&secret)?;
    fresh.require("messages:execute")?;
    fresh.require_account(&result.metadata.account_id)?;
    Ok(Json(json!({"data":result})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recipient {
    account_id: String,
    view_id: String,
    multi: bool,
}

pub(super) async fn reconcile(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Account>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let auth = secret.clone();
    let account = input.account_id.clone();
    let result = local_work(state.application.clone(), move |app| {
        app.reconcile_send(&app.authenticate(&auth)?, &input.account_id, &id)
    })
    .await?;
    let p = state.application.authenticate(&secret)?;
    p.require("operations:read")?;
    p.require_account(&account)?;
    Ok(Json(json!({"data":result})))
}

pub(super) async fn prepare_recipient(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<Recipient>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let auth = secret.clone();
    let account = input.account_id.clone();
    let data = local_work(state.application.clone(), move |app| {
        let p = app.authenticate(&auth)?;
        p.require("messages:execute")?;
        p.require_account(&input.account_id)?;
        app.prepare_recipient_local(
            &input.account_id,
            werc_application::recipients::RecipientSelection {
                view_id: input.view_id,
                multi: input.multi,
            },
        )
    })
    .await?;
    let fresh = state.application.authenticate(&secret)?;
    fresh.require("messages:execute")?;
    fresh.require_account(&account)?;
    Ok(Json(json!({"data":data})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StreamQuery {
    account_id: String,
    #[serde(default)]
    after: i64,
}

fn resume_cursor(headers: &HeaderMap, after: i64) -> Result<i64, AppError> {
    let cursor = match headers.get("last-event-id") {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| AppError::new("INVALID_ARGUMENT", "Last-Event-ID 必须是非负消息位置"))?,
        None => after,
    };
    if cursor < 0 {
        return Err(AppError::new("INVALID_ARGUMENT", "消息位置必须非负"));
    }
    Ok(cursor)
}

pub(super) async fn message_stream(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let principal = state.application.authenticate(&secret)?;
    principal.require("messages:read")?;
    principal.require_account(&query.account_id)?;
    let mut cursor = resume_cursor(&headers, query.after)?;
    // Read/decrypt off the async executor, both on initial connection and every page.
    let stream = async_stream::stream! {
        let mut last_gap = None;
        loop {
            let application = state.application.clone();
            let token = secret.clone();
            let account = query.account_id.clone();
            let batch = tokio::task::spawn_blocking(move || {
                let p = application.authenticate(&token)?;
                let rows = application.received_texts(&p, &account, cursor, 100)?;
                let progress = application.receive_cursor_local(&account)?;
                let health = application.receive_health_local(&account)?;
                let p = application.authenticate(&token)?;
                p.require("messages:read")?;
                p.require_account(&account)?;
                Ok::<_, AppError>((rows, progress, health))
            }).await.unwrap_or_else(|_| Err(AppError::internal()));
            match batch {
                Ok((rows, progress, health)) => {
                    let full = rows.len() == 100;
                    let gap = json!({"epoch":progress.as_ref().map(|p|&p.epoch),"gaps":progress.as_ref().map(|p|p.gaps),"dropped":progress.as_ref().map(|p|p.dropped),"historical_complete":false,"health":health});
                    if last_gap.as_ref() != Some(&gap) {
                        yield Ok(SseEvent::default().event("receive.status").data(gap.to_string()));
                        last_gap = Some(gap);
                    }
                    for row in rows {
                        // Authorization can be revoked while a slow subscriber drains a page.
                        let access = state.application.authenticate(&secret).and_then(|p| {
                            p.require("messages:read")?;
                            p.require_account(&query.account_id)
                        });
                        if let Err(error) = access {
                            yield Ok(SseEvent::default().event("stream.closed").data(json!({"code":error.code}).to_string()));
                            return;
                        }
                        match serde_json::to_string(&row) {
                            Ok(body) => {
                                cursor = row.position;
                                yield Ok(SseEvent::default().id(cursor.to_string()).event("message.received").data(body));
                            }
                            Err(_) => {
                                yield Ok(SseEvent::default().event("stream.closed").data("{\"code\":\"SERIALIZATION_FAILED\"}"));
                                return;
                            }
                        }
                    }
                    if !full { tokio::time::sleep(Duration::from_millis(500)).await; }
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_resume_uses_message_positions_and_rejects_negative_or_malformed_ids() {
        let mut headers = HeaderMap::new();
        assert_eq!(resume_cursor(&headers, 42).unwrap(), 42);
        assert!(resume_cursor(&headers, -1).is_err());
        headers.insert("last-event-id", "17".parse().unwrap());
        assert_eq!(resume_cursor(&headers, 42).unwrap(), 17);
        headers.insert("last-event-id", "-1".parse().unwrap());
        assert!(resume_cursor(&headers, 0).is_err());
        headers.insert("last-event-id", "abc".parse().unwrap());
        assert!(resume_cursor(&headers, 0).is_err());
    }
}
