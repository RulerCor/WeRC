//! On-demand message windows for an exact local username. No send authorization.
use crate::{
    Application,
    live_view::{LiveDirection, LivePage, LiveReadView},
};
use serde::{Deserialize, Serialize};
use werc_domain::{
    AppError, AppResult,
    live::{LiveConversationSelection, LiveMessageQuery},
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationCommand {
    OpenSearch {
        parent_view_id: String,
        result_token: String,
    },
    Open {
        parent_view_id: String,
        username: String,
    },
    Page {
        view_id: String,
        cursor: String,
        direction: LiveDirection,
    },
    Continue {
        view_id: String,
        cursor: String,
    },
    Find {
        view_id: String,
        query: LiveMessageQuery,
    },
    Context {
        view_id: String,
        local_id: String,
        source: Option<String>,
    },
    Clear {
        view_id: String,
    },
}
#[derive(Serialize)]
pub struct ConversationPage {
    pub focus: Option<ConversationFocus>,
    pub query: Option<LiveMessageQuery>,
    pub cards: Vec<crate::conversation_card::ConversationCard>,
    pub media: Vec<crate::conversation_media::ConversationMediaHint>,
    pub username: String,
    pub messages: LivePage,
    pub source_scope: &'static str,
    pub sending_enabled: bool,
}
#[derive(Clone, Serialize)]
pub struct ConversationFocus {
    pub local_id: String,
    pub source: Option<String>,
}
pub(crate) struct ConversationResult {
    pub(crate) focus: Option<ConversationFocus>,
    pub(crate) query: Option<LiveMessageQuery>,
    pub(crate) account: String,
    pub(crate) database: Zeroizing<String>,
    pub(crate) username: Zeroizing<String>,
    pub(crate) view: LiveReadView,
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "会话窗口已变化或过期，请重新打开")
}
// Application message metadata can contain media transport keys. This read
// surface exposes text and type placeholders, never those download descriptors.
fn public_page(
    mut page: LivePage,
) -> (
    LivePage,
    Vec<crate::conversation_media::ConversationMediaHint>,
    Vec<crate::conversation_card::ConversationCard>,
) {
    let media = page
        .rows
        .iter()
        .filter_map(|row| crate::conversation_media::hint(row, None))
        .collect();
    let cards = page
        .rows
        .iter()
        .filter_map(|row| crate::conversation_card::card(row, None))
        .collect();
    use zeroize::Zeroize;
    for row in &mut page.rows {
        row.metadata.zeroize();
    }
    (page, media, cards)
}
impl Application {
    pub fn live_conversation_local(
        &self,
        account: &str,
        command: ConversationCommand,
        request_id: &str,
    ) -> AppResult<Option<ConversationPage>> {
        // Clearing an owned view also invalidates its pending replacement. A
        // caller without a matching view cannot cancel another caller's read.
        if let ConversationCommand::Clear { view_id } = command {
            let mut slot = self
                .conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let Some(r) = slot.as_ref() else {
                return Ok(None);
            };
            if r.account != account || !r.view.matches_view(&view_id) {
                return Err(stale());
            }
            self.conversation_generation
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            slot.take();
            return Ok(None);
        }
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.exclusive(|| lease.check())?;
        lease.check()?;
        if let ConversationCommand::Page {
            view_id,
            cursor,
            direction,
        } = command
        {
            let slot = self
                .conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let r = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let binding = self.conversation_binding(account, &r.database, &r.username)?;
            r.view.check_view(&binding, &view_id)?;
            let (messages, media, cards) =
                public_page(r.view.page(&binding, Some((&cursor, direction)), 60)?);
            lease.check()?;
            return Ok(Some(ConversationPage {
                focus: r.focus.clone(),
                query: r.query.clone(),
                username: r.username.to_string(),
                messages,
                media,
                cards,
                source_scope: "selected_message_source",
                sending_enabled: false,
            }));
        }
        // Capture before parent/source validation so a concurrent clear also
        // retires reads that have not entered the provider yet.
        let generation = self
            .conversation_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let mut focus = None;
        let (database, selection, expected_binding) = match command {
            ConversationCommand::Open {
                parent_view_id,
                username,
            } => {
                let selection = LiveConversationSelection {
                    username,
                    before: None,
                    include_anchor: false,
                    newer_context: false,
                    query: None,
                };
                if !selection.valid() {
                    return Err(AppError::new("LIVE_CONVERSATION_INVALID", "会话标识无效"));
                }
                let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
                let parent = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.live_read_binding(account, &parent.database)?.0;
                parent.view.check_view(&binding, &parent_view_id)?;
                let database = parent.database.clone();
                let expected =
                    self.conversation_binding(account, &database, &selection.username)?;
                (database, selection, expected)
            }
            ConversationCommand::OpenSearch {
                parent_view_id,
                result_token,
            } => {
                let (username, anchor, _) =
                    self.search_hit(account, &parent_view_id, &result_token, false)?;
                let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
                let parent = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.live_read_binding(account, &parent.database)?.0;
                parent.view.check_view(&binding, &parent_view_id)?;
                let database = parent.database.clone();
                let expected = self.conversation_binding(account, &database, &username)?;
                focus = Some(ConversationFocus {
                    local_id: anchor.local_id.clone(),
                    source: None,
                });
                (
                    database,
                    LiveConversationSelection {
                        username,
                        before: Some(anchor),
                        include_anchor: true,
                        newer_context: false,
                        query: None,
                    },
                    expected,
                )
            }
            ConversationCommand::Continue { view_id, cursor } => {
                let slot = self
                    .conversation_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let r = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.conversation_binding(account, &r.database, &r.username)?;
                r.view.check_view(&binding, &view_id)?;
                let before = r.view.continuation_anchor(&binding, &cursor)?;
                (
                    r.database.clone(),
                    LiveConversationSelection {
                        username: r.username.to_string(),
                        before: Some(before),
                        include_anchor: false,
                        newer_context: false,
                        query: r.query.clone(),
                    },
                    binding,
                )
            }
            ConversationCommand::Find { view_id, query } => {
                if !query.valid() {
                    return Err(AppError::new(
                        "LIVE_QUERY_INVALID",
                        "请检查日期范围、消息类型和搜索文字（最多256个UTF-8字节）",
                    ));
                }
                let slot = self
                    .conversation_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let r = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.conversation_binding(account, &r.database, &r.username)?;
                r.view.check_view(&binding, &view_id)?;
                (
                    r.database.clone(),
                    LiveConversationSelection {
                        username: r.username.to_string(),
                        before: None,
                        include_anchor: false,
                        newer_context: false,
                        query: Some(query),
                    },
                    binding,
                )
            }
            ConversationCommand::Context {
                view_id,
                local_id,
                source,
            } => {
                if source.is_some() {
                    return Err(stale());
                }
                let slot = self
                    .conversation_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let r = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.conversation_binding(account, &r.database, &r.username)?;
                let window = r.view.recovery_window(&binding, &view_id)?;
                let row = window
                    .rows
                    .iter()
                    .find(|row| row.local_id == local_id)
                    .ok_or_else(stale)?;
                focus = Some(ConversationFocus {
                    local_id: local_id.clone(),
                    source: None,
                });
                (
                    r.database.clone(),
                    LiveConversationSelection {
                        username: r.username.to_string(),
                        before: Some(werc_domain::live::LiveAnchor {
                            timestamp: row.timestamp,
                            local_id,
                            server_id: row.server_id.clone(),
                        }),
                        include_anchor: true,
                        newer_context: false,
                        query: None,
                    },
                    binding,
                )
            }
            _ => return Err(stale()),
        };
        let (_, salt, source) = self.live_read_binding(account, &database)?;
        let keys = self.live_keys_local(account)?;
        let key = crate::keyring::key_for(&keys, &salt)?;
        let mut window = self.provider.read_live_conversation(
            account,
            &database,
            &key,
            &source,
            &selection,
            &lease.cancel,
        )?;
        lease.check()?;
        if selection
            .query
            .as_ref()
            .is_some_and(|q| window.rows.iter().any(|row| !q.matches(row)))
        {
            return Err(AppError::new(
                "LIVE_PROTOCOL_INVALID",
                "读取结果不符合本次查找条件",
            ));
        }
        if focus.is_some() {
            let mut newer_selection = selection.clone();
            newer_selection.include_anchor = false;
            newer_selection.newer_context = true;
            let newer = self.provider.read_live_conversation(
                account,
                &database,
                &key,
                &source,
                &newer_selection,
                &lease.cancel,
            )?;
            lease.check()?;
            validate_newer_context(&newer, &newer_selection)?;
            let keep = werc_domain::live::LIVE_WINDOW_MAX_ROWS - newer.rows.len();
            window.has_older |= window.rows.len() > keep;
            window.rows.truncate(keep);
            window.rows.splice(0..0, newer.rows);
        }
        if self.conversation_binding(account, &database, &selection.username)? != expected_binding {
            return Err(stale());
        }
        let view = LiveReadView::new(&expected_binding, window)?;
        let (messages, media, cards) = public_page(view.page(&expected_binding, None, 60)?);
        if focus
            .as_ref()
            .is_some_and(|f| !messages.rows.iter().any(|row| row.local_id == f.local_id))
        {
            return Err(stale());
        }
        let mut slot = self
            .conversation_result
            .lock()
            .map_err(|_| AppError::internal())?;
        lease.check()?;
        if self
            .conversation_generation
            .load(std::sync::atomic::Ordering::SeqCst)
            != generation
        {
            return Err(AppError::new("QUERY_CANCELED", "会话读取已取消"));
        }
        *slot = Some(ConversationResult {
            focus: focus.clone(),
            query: selection.query.clone(),
            account: account.into(),
            database,
            username: Zeroizing::new(selection.username.clone()),
            view,
        });
        Ok(Some(ConversationPage {
            focus,
            query: selection.query.clone(),
            username: selection.username.clone(),
            messages,
            media,
            cards,
            source_scope: "selected_message_source",
            sending_enabled: false,
        }))
    }
    pub(crate) fn conversation_binding(
        &self,
        account: &str,
        database: &str,
        username: &str,
    ) -> AppResult<String> {
        let binding = self.live_read_binding(account, database)?.0;
        self.send_digest(
            account,
            "live-conversation-context-v1",
            &(binding, username),
        )
    }
}

/// Also check provider implementations, not only the worker transport.
pub(crate) fn validate_newer_context(
    window: &werc_domain::live::LiveWindow,
    selection: &LiveConversationSelection,
) -> AppResult<()> {
    drop(crate::live_view::encode_window(window)?);
    let boundary = selection.before.as_ref().and_then(|a| a.key());
    if window.rows.len() > 30
        || window.has_older
        || window.rows.iter().any(|row| {
            boundary.is_some_and(|b| (row.timestamp, row.local_id.parse::<i64>().unwrap_or(0)) <= b)
                || selection.query.as_ref().is_some_and(|q| !q.matches(row))
        })
    {
        return Err(AppError::new(
            "LIVE_PROTOCOL_INVALID",
            "更新上下文不符合本次消息边界",
        ));
    }
    Ok(())
}
