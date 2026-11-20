//! Independent copies of exactly matched revoke notices. Never changes WeChat
//! or filters the receive stream. Originals not observed locally remain missing.
use crate::Application;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use werc_domain::{AppError, AppResult, library::DocumentConnection};
const STATE: &str = "message-preservation-v1";
const KIND: &str = "message_preserved";
#[derive(Default, Serialize, Deserialize)]
struct Settings {
    login: String,
    conversations: BTreeSet<String>,
    after: i64,
    unrecognized_notices: u64,
}
#[derive(Serialize, Deserialize)]
struct Original {
    observed_at: String,
    message: werc_domain::receive::ReceivedText,
}
#[derive(Serialize, Deserialize)]
struct Saved {
    login: String,
    conversation: String,
    original_server_id: String,
    notice_at: String,
    notice_position: i64,
    original: Option<Original>,
}
#[derive(Serialize)]
pub struct PreservationStatus {
    pub conversations: Vec<String>,
    pub scanned_position: i64,
    pub saved: u64,
    pub unrecognized_notices: u64,
}
#[derive(Serialize)]
pub struct PreservedMessage {
    pub id: String,
    pub conversation: String,
    pub original_server_id: String,
    pub notice_at: String,
    pub notice_position: i64,
    pub state: &'static str,
    pub message_type: Option<u32>,
    pub text: Option<String>,
    pub original_observed_at: Option<String>,
    pub card: Option<werc_domain::message_card::MessageCard>,
}
#[derive(Serialize)]
pub struct PreservedPage {
    pub data: Vec<PreservedMessage>,
    pub next_cursor: Option<i64>,
    pub scanned: usize,
}
impl Application {
    fn message_preservation_settings(&self, account: &str) -> AppResult<(i64, Settings)> {
        match self.open_document(account, STATE, "message_preservation") {
            Ok((header, state)) => Ok((header.revision, state)),
            Err(e) if e.code == "NOT_FOUND" => Ok((0, Settings::default())),
            Err(e) => Err(e),
        }
    }
    pub fn message_preservation_status_local(
        &self,
        account: &str,
    ) -> AppResult<PreservationStatus> {
        let (_, state) = self.message_preservation_settings(account)?;
        Ok(PreservationStatus {
            conversations: state.conversations.into_iter().collect(),
            scanned_position: state.after,
            saved: self.repository.library_count(account, KIND)?,
            unrecognized_notices: state.unrecognized_notices,
        })
    }
    pub fn configure_message_preservation_local(
        &self,
        account: &str,
        conversation: &str,
        enabled: bool,
    ) -> AppResult<PreservationStatus> {
        if !werc_domain::live::valid_live_username(conversation) {
            return Err(AppError::new("INVALID_ARGUMENT", "请选择精确会话账号"));
        }
        let _guard = self
            .message_preservation_guard
            .lock()
            .map_err(|_| AppError::internal())?;
        let (revision, mut state) = self.message_preservation_settings(account)?;
        let connection = if enabled {
            let (context, revision) = self.checked_send_context(account, "filehelper")?;
            if !state.login.is_empty() && state.login != context.sender_id {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "保留设置属于其他微信登录身份",
                ));
            }
            state.login = context.sender_id;
            if state.conversations.insert(conversation.into()) {
                state.after = 0;
                state.unrecognized_notices = 0;
            }
            if state.conversations.len() > 4096 {
                return Err(AppError::new("QUOTA_EXCEEDED", "保留会话超过4096项"));
            }
            Some(DocumentConnection {
                revision,
                key_revision: self.repository.key_revision(account)?,
            })
        } else {
            state.conversations.remove(conversation);
            None
        };
        let document =
            self.seal_document(account, STATE, "message_preservation", revision + 1, &state)?;
        self.repository
            .library_save(&[document], connection.as_ref())?;
        self.message_preservation_status_local(account)
    }
    /// The observation archive is already durable before this independent sweep.
    /// A failed sweep leaves its checkpoint unchanged, without blocking native ACK.
    pub fn step_message_preservation_local(&self, account: &str) -> AppResult<PreservationStatus> {
        let _guard = self
            .message_preservation_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "正在保留撤回记录"))?;
        let (revision, mut state) = self.message_preservation_settings(account)?;
        if state.conversations.is_empty() {
            return self.message_preservation_status_local(account);
        }
        let rows = self.received_texts_local(account, state.after, 100)?;
        if rows.is_empty() {
            return self.message_preservation_status_local(account);
        }
        let mut docs = Vec::new();
        let mut seen = BTreeSet::new();
        for row in rows {
            state.after = row.position;
            let payload = row.payload;
            if payload.sender_id != state.login || !state.conversations.contains(&payload.target_id)
            {
                continue;
            }
            if !matches!(payload.message.message_type, 10000 | 10002) {
                // A previously missing original may be observed after its notice.
                let server = &payload.message.native_message_id;
                if server != "0" {
                    let id = format!(
                        "revoked-{}",
                        self.send_digest(
                            account,
                            "preserved-revoke-id",
                            &(&state.login, &payload.target_id, server)
                        )?
                    );
                    match self.open_document::<Saved>(account, &id, KIND) {
                        Ok((header, mut saved))
                            if saved.original.is_none() && seen.insert(id.clone()) =>
                        {
                            saved.original = Some(Original {
                                observed_at: payload.observed_at,
                                message: payload.message,
                            });
                            docs.push(self.seal_document(
                                account,
                                &id,
                                KIND,
                                header.revision + 1,
                                &saved,
                            )?);
                        }
                        Ok(_) => {}
                        Err(e) if e.code == "NOT_FOUND" => {}
                        Err(e) => return Err(e),
                    }
                }
                continue;
            }
            let detail = payload.message.metadata.as_deref().and_then(|m| {
                werc_domain::message_card::records::extract(payload.message.message_type, m)
                    .ok()
                    .flatten()
            });
            let Some(detail) =
                detail.filter(|d| d.kind == werc_domain::message_card::records::RecordKind::Revoke)
            else {
                if payload.message.message_type == 10002 {
                    state.unrecognized_notices = state.unrecognized_notices.saturating_add(1);
                }
                continue;
            };
            let Some(server) = detail.fields.get("原消息编号").filter(|s| {
                s.parse::<u64>()
                    .ok()
                    .is_some_and(|n| n > 0 && n.to_string() == **s)
            }) else {
                state.unrecognized_notices = state.unrecognized_notices.saturating_add(1);
                continue;
            };
            if detail
                .fields
                .get("原会话")
                .is_some_and(|s| s != &payload.target_id)
            {
                state.unrecognized_notices = state.unrecognized_notices.saturating_add(1);
                continue;
            }
            let id = format!(
                "revoked-{}",
                self.send_digest(
                    account,
                    "preserved-revoke-id",
                    &(&state.login, &payload.target_id, server)
                )?
            );
            if !seen.insert(id.clone()) {
                continue;
            }
            let existing = match self.open_document::<Saved>(account, &id, KIND) {
                Ok((h, s)) => Some((h.revision, s)),
                Err(e) if e.code == "NOT_FOUND" => None,
                Err(e) => return Err(e),
            };
            if existing.as_ref().is_some_and(|(_, s)| s.original.is_some()) {
                continue;
            }
            let original_id = self.send_digest(
                account,
                "receive-native-id",
                &(&state.login, &payload.target_id, server),
            )?;
            let original = self
                .repository
                .received_text_by_id(account, &original_id)?
                .map(|r| self.decrypt_received_text(account, r))
                .transpose()?
                .filter(|p| {
                    p.sender_id == state.login
                        && p.target_id == payload.target_id
                        && p.message.native_message_id == *server
                        && !matches!(p.message.message_type, 10000 | 10002)
                })
                .map(|p| Original {
                    observed_at: p.observed_at,
                    message: p.message,
                });
            let (version, saved) = if let Some((version, mut saved)) = existing {
                if original.is_none() {
                    continue;
                }
                saved.original = original;
                (version + 1, saved)
            } else {
                (
                    1,
                    Saved {
                        login: state.login.clone(),
                        conversation: payload.target_id,
                        original_server_id: server.clone(),
                        notice_at: payload.observed_at,
                        notice_position: row.position,
                        original,
                    },
                )
            };
            docs.push(self.seal_document(account, &id, KIND, version, &saved)?);
        }
        docs.push(self.seal_document(
            account,
            STATE,
            "message_preservation",
            revision + 1,
            &state,
        )?);
        self.repository.library_save(&docs, None)?;
        self.message_preservation_status_local(account)
    }
    pub fn preserved_messages_local(
        &self,
        account: &str,
        before: Option<i64>,
        conversation: Option<&str>,
        state: Option<&str>,
    ) -> AppResult<PreservedPage> {
        self.ensure_account(account)?;
        if conversation.is_some_and(|s| !werc_domain::live::valid_live_username(s))
            || state.is_some_and(|s| !matches!(s, "original_saved" | "original_not_observed"))
        {
            return Err(AppError::new("INVALID_ARGUMENT", "会话或保留状态无效"));
        }
        let page = self.repository.library_list(account, KIND, before, 50)?;
        let scanned = page.data.len();
        let mut data = Vec::new();
        for header in page.data {
            let (_, saved): (_, Saved) = self.open_document(account, &header.id, KIND)?;
            let status = if saved.original.is_some() {
                "original_saved"
            } else {
                "original_not_observed"
            };
            if conversation.is_some_and(|c| c != saved.conversation)
                || state.is_some_and(|s| s != status)
            {
                continue;
            }
            let card = saved.original.as_ref().and_then(|o| {
                o.message.metadata.as_deref().and_then(|body| {
                    werc_domain::message_card::parse_for_type(o.message.message_type, body)
                        .ok()
                        .flatten()
                })
            });
            data.push(PreservedMessage {
                id: header.id,
                conversation: saved.conversation,
                original_server_id: saved.original_server_id,
                notice_at: saved.notice_at,
                notice_position: saved.notice_position,
                state: status,
                message_type: saved.original.as_ref().map(|o| o.message.message_type),
                text: saved
                    .original
                    .as_ref()
                    .filter(|o| o.message.message_type == 1)
                    .map(|o| o.message.text.clone()),
                original_observed_at: saved.original.as_ref().map(|o| o.observed_at.clone()),
                card,
            });
        }
        Ok(PreservedPage {
            data,
            next_cursor: page.next_cursor,
            scanned,
        })
    }
}
