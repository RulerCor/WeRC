//! Quote sources are resolved from an authenticated view or an authenticated receive record.
use crate::{Application, conversation_media::ConversationMediaSelection};
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, Principal, outbox::*};
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientQuote {
    pub username: String,
    pub server_id: Option<String>,
    pub selection: Option<ConversationMediaSelection>,
    pub position: Option<i64>,
    pub text: String,
    pub idempotency_key: String,
}
impl Application {
    pub fn create_quote_send(
        &self,
        p: &Principal,
        account: &str,
        input: &ClientQuote,
    ) -> AppResult<SendStatus> {
        p.require("messages:send")?;
        p.require("messages:read")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let (None, Some(position)) = (&input.selection, input.position) {
            return self.create_observed_quote_send(p, account, input, position);
        }
        let (Some(selection), Some(server_id), None) =
            (&input.selection, &input.server_id, input.position)
        else {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "请选择 position 或 selection；selection 需要 server_id",
            ));
        };
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            &input.username,
            &input.idempotency_key,
            crate::outbox::ClientContent::Quote(&input.text, server_id),
        )? {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, &input.username)?;
        let draft = self.prepare_quote_send(
            &p.id,
            account,
            selection,
            &input.text,
            &input.idempotency_key,
            Some((&input.username, server_id)),
        )?;
        self.finish_client_send(p, draft)
    }
    fn create_observed_quote_send(
        &self,
        p: &Principal,
        account: &str,
        input: &ClientQuote,
        position: i64,
    ) -> AppResult<SendStatus> {
        if position <= 0 {
            return Err(AppError::new("INVALID_ARGUMENT", "position 必须是正整数"));
        }
        validate_reply(&input.text, &input.idempotency_key)?;
        self.ensure_current_connection(account)?;
        let invalid = || {
            AppError::new(
                "QUOTE_SOURCE_INVALID",
                "该位置没有属于当前账号及目标会话、带完整服务器标识的普通文字来信",
            )
        };
        let row = self
            .received_texts_local(account, position - 1, 1)?
            .into_iter()
            .find(|row| row.position == position)
            .ok_or_else(invalid)?;
        let payload = &row.payload;
        let route = payload.route.as_ref().ok_or_else(invalid)?;
        if route.sender == payload.sender_id
            || route.receiver != payload.sender_id
            || route.conversation != input.username
            || payload.target_id != input.username
            || payload.message.message_type != 1
            || input
                .server_id
                .as_ref()
                .is_some_and(|id| *id != payload.message.native_message_id)
        {
            return Err(invalid());
        }
        let original = observed_quote_body(&route.conversation, &payload.message.text);
        let source = make_source(&payload.message.native_message_id, original);
        if !source.valid() {
            return Err(invalid());
        }
        self.prepare_client_target_if_needed(p, account, &input.username)?;
        let (context, revision) = self.checked_send_context(account, &input.username)?;
        if context.sender_id != payload.sender_id {
            return Err(invalid());
        }
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            &input.username,
            &input.idempotency_key,
            crate::outbox::ClientContent::Quote(&input.text, &source.server_id),
        )? {
            return Ok(existing);
        }
        // Draft insertion binds the connection revision and idempotency key in
        // the repository transaction. Only native execution must be serialized.
        let draft = self.persist_send(
            &p.id,
            account,
            &input.idempotency_key,
            revision,
            quote_payload(context, &input.text, source),
        )?;
        self.finish_client_send(p, draft)
    }

    pub fn create_quote_send_local(
        &self,
        account: &str,
        selection: &ConversationMediaSelection,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_quote_send("desktop", account, selection, text, key, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_quote_send(
        &self,
        creator: &str,
        account: &str,
        selection: &ConversationMediaSelection,
        text: &str,
        key: &str,
        expected: Option<(&str, &str)>,
    ) -> AppResult<SendStatus> {
        validate_reply(text, key)?;
        let _guard = self
            .send_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "发送任务正在处理"))?;
        self.with_conversation_message(account, selection, |selected| {
            let row = selected.row;
            if expected.is_some_and(|(target, server_id)| {
                selected.username.as_str() != target || row.server_id.as_deref() != Some(server_id)
            }) {
                return Err(AppError::new(
                    "QUOTE_SOURCE_INVALID",
                    "引用消息的会话或服务器标识与请求不符",
                ));
            }
            let source = make_source(
                row.server_id.as_deref().unwrap_or_default(),
                row.text.as_deref().unwrap_or_default(),
            );
            if !matches!(row.content_status.as_str(), "text" | "text_zstd")
                || row.message_type != "1"
                || !source.valid()
            {
                return Err(AppError::new(
                    "QUOTE_SOURCE_INVALID",
                    "请选择有服务端标识、正文完整的普通文字消息",
                ));
            }
            let (context, revision) = self.checked_send_context(account, &selected.username)?;
            self.persist_send(
                creator,
                account,
                key,
                revision,
                quote_payload(context, text, source),
            )
        })
    }
}

fn validate_reply(text: &str, key: &str) -> AppResult<()> {
    if !valid_send_text(text)
        || !(16..=128).contains(&key.len())
        || !key.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "需要1至10000个UTF-8字节的回复和16至128个可打印ASCII字符的幂等键",
        ));
    }
    Ok(())
}

fn observed_quote_body<'a>(conversation: &str, raw: &'a str) -> &'a str {
    // The receive hook preserves WeChat's group sender envelope; the native
    // quote descriptor exposes the body without it. Only normalize one outer
    // envelope on an authenticated incoming group event. This member field is
    // never used to resolve the message or as proof of its author's identity.
    if conversation.ends_with("@chatroom") {
        if let Some((member, body)) = raw.split_once(":\n") {
            if !member.is_empty()
                && member.len() < 128
                && !member.ends_with("@chatroom")
                && member
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'@'))
            {
                return body;
            }
        }
    }
    raw
}

fn make_source(server_id: &str, original: &str) -> QuoteSource {
    QuoteSource {
        server_id: server_id.into(),
        text: original.into(),
        sha256: hex::encode(Sha256::digest(original.as_bytes())),
    }
}

fn quote_payload(context: SendContext, text: &str, source: QuoteSource) -> TextSendPayload {
    TextSendPayload {
        moment: None,
        context,
        text: text.into(),
        quote: Some(source),
        mentions: Vec::new(),
        voice: None,
        image: None,
        file: None,
        video: None,
        emoticon: None,
    }
}
