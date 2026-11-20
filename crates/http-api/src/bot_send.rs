use super::*;
use werc_application::{mention_send::ClientMention, quote_send::ClientQuote};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MentionInput {
    account_id: String,
    message: ClientMention,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QuoteInput {
    account_id: String,
    message: ClientQuote,
}

pub(super) async fn mention(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<MentionInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let auth = secret.clone();
    let result = local_work(state.application.clone(), move |app| {
        app.create_mention_send(&app.authenticate(&auth)?, &input.account_id, &input.message)
    })
    .await?;
    verify_result(&state, &secret, &result, false)?;
    Ok(Json(json!({"data":result})))
}
pub(super) async fn quote(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(input): Json<QuoteInput>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = bearer(&headers)?.to_owned();
    let auth = secret.clone();
    let result = local_work(state.application.clone(), move |app| {
        app.create_quote_send(&app.authenticate(&auth)?, &input.account_id, &input.message)
    })
    .await?;
    verify_result(&state, &secret, &result, true)?;
    Ok(Json(json!({"data":result})))
}
fn verify_result(
    state: &ApiState,
    secret: &str,
    result: &werc_domain::outbox::SendStatus,
    quote: bool,
) -> Result<(), AppError> {
    let p = state.application.authenticate(secret)?;
    p.require("messages:send")?;
    p.require("operations:create")?;
    if quote {
        p.require("messages:read")?;
    }
    p.require_account(&result.metadata.account_id)
}
