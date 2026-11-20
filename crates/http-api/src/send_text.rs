use super::*;
use werc_domain::outbox::SendState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SendText {
    account_id: String,
    username: String,
    text: String,
    #[serde(default = "default_wait")]
    wait_seconds: u16,
}
fn default_wait() -> u16 {
    30
}
fn pending(state: SendState) -> bool {
    matches!(
        state,
        SendState::AwaitingApproval | SendState::Queued | SendState::Submitting
    )
}

pub(super) async fn send_text(
    State(state): State<ApiState>,
    axum::Extension(principal): axum::Extension<Principal>,
    headers: HeaderMap,
    Json(input): Json<SendText>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok());
    if !key
        .is_some_and(|s| (16..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic()))
        || input.username.trim().is_empty()
        || input.username.len() > 256
        || !werc_domain::outbox::valid_send_text(&input.text)
        || input.wait_seconds > 120
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "需要合法幂等键、目标、1–10000 UTF-8字节文字及0–120整数秒等待时间",
        )
        .into());
    }
    principal.require("messages:send")?;
    principal.require("operations:create")?;
    principal.require_account(&input.account_id)?;
    // Admission precedes persistence. A rejected wait request creates no draft.
    let _permit = if input.wait_seconds > 0 {
        Some(
            state
                .send_waiters
                .clone()
                .try_acquire_owned()
                .map_err(|_| {
                    AppError::new(
                        "BUSY",
                        "同时最多等待16个发送结果；可用wait_seconds=0立即返回草稿",
                    )
                })?,
        )
    } else {
        None
    };
    let until = tokio::time::Instant::now() + Duration::from_secs(u64::from(input.wait_seconds));
    let key = key.unwrap_or_default().to_owned();
    let secret = bearer(&headers)?.to_owned();
    let account = input.account_id.clone();
    let create_secret = secret.clone();
    let mut status = local_work(state.application.clone(), move |app| {
        let current = app.authenticate(&create_secret)?;
        app.create_text_send(
            &current,
            &input.account_id,
            &input.username,
            &input.text,
            &key,
        )
    })
    .await?;
    loop {
        let current = state.application.authenticate(&secret)?;
        current.require("messages:send")?;
        current.require("operations:create")?;
        current.require_account(&account)?;
        if !pending(status.state) || tokio::time::Instant::now() >= until {
            return Ok(Json(json!({"data":status})));
        }
        tokio::time::sleep_until(
            until.min(tokio::time::Instant::now() + Duration::from_millis(250)),
        )
        .await;
        let read_secret = secret.clone();
        let read_account = account.clone();
        let id = status.metadata.id.clone();
        status = local_work(state.application.clone(), move |app| {
            let current = app.authenticate(&read_secret)?;
            app.created_text_send_status(&current, &read_account, &id)
        })
        .await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_wait_and_nonpending_outcomes_are_explicit() {
        let input: SendText =
            serde_json::from_value(json!({"account_id":"a","username":"u","text":"t"})).unwrap();
        assert_eq!(input.wait_seconds, 30);
        for state in [
            SendState::Unknown,
            SendState::ServerAccepted,
            SendState::RecipientConfirmed,
            SendState::Rejected,
            SendState::Canceled,
            SendState::Expired,
        ] {
            assert!(!pending(state));
        }
        for state in [
            SendState::AwaitingApproval,
            SendState::Queued,
            SendState::Submitting,
        ] {
            assert!(pending(state));
        }
    }
}
