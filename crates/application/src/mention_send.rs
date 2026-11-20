use crate::{Application, recipients::RecipientSelection};
use serde::Deserialize;
use werc_domain::{AppError, AppResult, Principal, outbox::*};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientMention {
    pub username: String,
    pub members: Vec<String>,
    pub text: String,
    pub idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MentionSelection {
    pub recipient: RecipientSelection,
    pub parent_view_id: String,
    pub cursor: Option<String>,
    pub members: Vec<String>,
}
impl Application {
    pub fn create_mention_send(
        &self,
        p: &Principal,
        account: &str,
        input: &ClientMention,
    ) -> AppResult<SendStatus> {
        p.require("messages:send")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if !werc_domain::mentions::valid_members(&input.username, &input.members)
            || !valid_send_text(&input.text)
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "群提醒需要精确群账号、1至8个不同成员账号及有效文字；不支持昵称或@所有人",
            ));
        }
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            &input.username,
            &input.idempotency_key,
            crate::outbox::ClientContent::Mention(&input.text, &input.members),
        )? {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, &input.username)?;
        let (context, revision) = self.checked_send_context(account, &input.username)?;
        let draft = self.persist_send(
            &p.id,
            account,
            &input.idempotency_key,
            revision,
            TextSendPayload {
                context,
                text: input.text.clone(),
                mentions: input.members.clone(),
                moment: None,
                quote: None,
                voice: None,
                image: None,
                file: None,
                video: None,
                emoticon: None,
            },
        )?;
        self.finish_client_send(p, draft)
    }
    pub fn create_mention_send_local(
        &self,
        account: &str,
        selection: MentionSelection,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        let (target, _) = self.selected_recipient(account, &selection.recipient)?;
        if !werc_domain::mentions::valid_members(&target, &selection.members)
            || text.trim().is_empty()
            || text.len() > MAX_TEXT_BYTES
            || text.contains('\0')
            || !(16..=128).contains(&key.len())
            || !key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(AppError::new(
                "MENTION_SELECTION_INVALID",
                "请选择本页1至8位不同成员，文字需在10000个UTF-8字节内",
            ));
        }
        // The same prepared target epoch must survive the authenticated re-read.
        let (context, revision) = self.checked_send_context(account, &target)?;
        let page = self.live_group_members_local(
            account,
            &selection.parent_view_id,
            &target,
            selection.cursor.as_deref(),
            &uuid::Uuid::new_v4().to_string(),
        )?;
        if !selection.members.iter().all(|id| {
            page.rows
                .iter()
                .any(|row| row.anchor.username.as_ref() == Some(id))
        }) {
            return Err(AppError::new(
                "MENTION_MEMBER_CHANGED",
                "所选成员不在当前认证成员页，请重新读取后选择",
            ));
        }
        let _guard = self
            .send_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "发送任务正在处理"))?;
        if self.selected_recipient(account, &selection.recipient)?.0 != target
            || self.checked_send_context(account, &target)? != (context.clone(), revision)
        {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "选择的会话或发送身份已变化",
            ));
        }
        self.persist_send(
            "desktop",
            account,
            key,
            revision,
            TextSendPayload {
                moment: None,
                context,
                text: text.into(),
                mentions: selection.members,
                quote: None,
                voice: None,
                image: None,
                file: None,
                video: None,
                emoticon: None,
            },
        )
    }
}
