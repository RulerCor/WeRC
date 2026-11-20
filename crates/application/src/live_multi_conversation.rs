//! Exact-username reads over an explicitly prepared, authenticated source set.
use crate::{
    Application,
    live_conversation::{ConversationCommand, ConversationFocus},
    live_multi::{Source, budget, empty_window},
    live_multi_view::{MultiLivePage, MultiReadView, SourceCursor, WindowMerger},
};
use serde::Serialize;
use std::{sync::atomic::Ordering, time::Instant};
use werc_domain::{
    AppError, AppResult,
    live::{LiveConversationSelection, LiveMessageQuery},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Serialize)]
pub struct MultiConversationPage {
    pub focus: Option<ConversationFocus>,
    pub query: Option<LiveMessageQuery>,
    pub cards: Vec<crate::conversation_card::ConversationCard>,
    pub media: Vec<crate::conversation_media::ConversationMediaHint>,
    pub username: String,
    pub messages: MultiLivePage,
    pub source_scope: &'static str,
    pub sending_enabled: bool,
}
pub(crate) struct MultiConversationResult {
    pub(crate) context_ceiling: Option<(String, werc_domain::live::LiveAnchor)>,
    pub(crate) focus: Option<ConversationFocus>,
    pub(crate) query: Option<LiveMessageQuery>,
    pub(crate) account: String,
    pub(crate) sources: Vec<Source>,
    pub(crate) username: Zeroizing<String>,
    pub(crate) view: MultiReadView,
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "跨分片会话窗口已变化或过期，请重新打开")
}
fn public_page(
    mut page: MultiLivePage,
) -> (
    MultiLivePage,
    Vec<crate::conversation_media::ConversationMediaHint>,
    Vec<crate::conversation_card::ConversationCard>,
) {
    let media = page
        .rows
        .iter()
        .filter_map(|row| crate::conversation_media::hint(&row.message, Some(&row.source)))
        .collect();
    let cards = page
        .rows
        .iter()
        .filter_map(|row| crate::conversation_card::card(&row.message, Some(&row.source)))
        .collect();
    for row in &mut page.rows {
        row.message.metadata.zeroize();
    }
    (page, media, cards)
}
impl Application {
    pub(crate) fn multi_conversation_binding(
        &self,
        account: &str,
        sources: &[Source],
        username: &str,
    ) -> AppResult<String> {
        self.send_digest(
            account,
            "live-multi-conversation-context-v1",
            &(self.multi_context(account, sources)?, username),
        )
    }
    pub fn live_multi_conversation_local(
        &self,
        account: &str,
        command: ConversationCommand,
        request_id: &str,
    ) -> AppResult<Option<MultiConversationPage>> {
        if let ConversationCommand::Clear { view_id } = command {
            let mut slot = self
                .multi_conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let Some(result) = slot.as_ref() else {
                return Ok(None);
            };
            if result.account != account || !result.view.matches_view(&view_id) {
                return Err(stale());
            }
            self.multi_conversation_generation
                .fetch_add(1, Ordering::SeqCst);
            slot.take();
            return Ok(None);
        }
        let start = Instant::now();
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
                .multi_conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let r = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let binding = self.multi_conversation_binding(account, &r.sources, &r.username)?;
            r.view.check_view(&binding, &view_id)?;
            let (messages, media, cards) =
                public_page(r.view.page(&binding, Some((&cursor, direction)), 60)?);
            lease.check()?;
            return Ok(Some(MultiConversationPage {
                focus: r.focus.clone(),
                query: r.query.clone(),
                username: r.username.to_string(),
                messages,
                media,
                cards,
                source_scope: "selected_message_sources",
                sending_enabled: false,
            }));
        }
        let generation = self
            .multi_conversation_generation
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let mut focus = None;
        let mut context_anchor: Option<(String, werc_domain::live::LiveAnchor)> = None;
        let (sources, username, cursors, query) = match command {
            ConversationCommand::Open {
                parent_view_id,
                username,
            } => {
                if !(LiveConversationSelection {
                    username: username.clone(),
                    before: None,
                    include_anchor: false,
                    newer_context: false,
                    query: None,
                })
                .valid()
                {
                    return Err(AppError::new("LIVE_CONVERSATION_INVALID", "会话标识无效"));
                }
                let slot = self
                    .multi_live_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let parent = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.multi_context(account, &parent.sources)?;
                parent.view.check_view(&binding, &parent_view_id)?;
                let cursors = parent
                    .sources
                    .iter()
                    .map(|_| SourceCursor {
                        before: None,
                        more: true,
                    })
                    .collect::<Vec<_>>();
                (
                    parent.sources.clone(),
                    Zeroizing::new(username),
                    cursors,
                    None,
                )
            }
            ConversationCommand::OpenSearch {
                parent_view_id,
                result_token,
            } => {
                let (username, anchor, source) =
                    self.search_hit(account, &parent_view_id, &result_token, true)?;
                let source = source.ok_or_else(stale)?;
                let slot = self
                    .multi_live_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let parent = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.multi_context(account, &parent.sources)?;
                parent.view.check_view(&binding, &parent_view_id)?;
                focus = Some(ConversationFocus {
                    local_id: anchor.local_id.clone(),
                    source: Some(source.clone()),
                });
                context_anchor = Some((source, anchor));
                let cursors = parent
                    .sources
                    .iter()
                    .map(|_| SourceCursor {
                        before: None,
                        more: true,
                    })
                    .collect();
                (
                    parent.sources.clone(),
                    Zeroizing::new(username),
                    cursors,
                    None,
                )
            }
            ConversationCommand::Continue { view_id, cursor } => {
                let slot = self
                    .multi_conversation_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let r = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.multi_conversation_binding(account, &r.sources, &r.username)?;
                r.view.check_view(&binding, &view_id)?;
                context_anchor = r.context_ceiling.clone();
                (
                    r.sources.clone(),
                    r.username.clone(),
                    r.view.continuation(&binding, &cursor)?,
                    r.query.clone(),
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
                    .multi_conversation_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let r = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.multi_conversation_binding(account, &r.sources, &r.username)?;
                r.view.check_view(&binding, &view_id)?;
                let cursors = r
                    .sources
                    .iter()
                    .map(|_| SourceCursor {
                        before: None,
                        more: true,
                    })
                    .collect();
                (r.sources.clone(), r.username.clone(), cursors, Some(query))
            }
            ConversationCommand::Context {
                view_id,
                local_id,
                source,
            } => {
                let source = source.ok_or_else(stale)?;
                let slot = self
                    .multi_conversation_result
                    .lock()
                    .map_err(|_| AppError::internal())?;
                let r = slot
                    .as_ref()
                    .filter(|r| r.account == account)
                    .ok_or_else(stale)?;
                let binding = self.multi_conversation_binding(account, &r.sources, &r.username)?;
                let (rows, _) = r.view.recovery_window(&binding, &view_id)?;
                let row = rows
                    .iter()
                    .find(|row| row.source == source && row.message.local_id == local_id)
                    .ok_or_else(stale)?;
                focus = Some(ConversationFocus {
                    local_id: local_id.clone(),
                    source: Some(source.clone()),
                });
                context_anchor = Some((
                    source,
                    werc_domain::live::LiveAnchor {
                        timestamp: row.message.timestamp,
                        local_id,
                        server_id: row.message.server_id.clone(),
                    },
                ));
                let cursors = r
                    .sources
                    .iter()
                    .map(|_| SourceCursor {
                        before: None,
                        more: true,
                    })
                    .collect();
                (r.sources.clone(), r.username.clone(), cursors, None)
            }
            _ => return Err(stale()),
        };
        let binding = self.multi_conversation_binding(account, &sources, &username)?;
        let keys = self.live_keys_local(account)?;
        let mut merger = WindowMerger::new();
        let mut newer_rows = Vec::new();
        if sources.len() != cursors.len() {
            return Err(stale());
        }
        for (index, (source, cursor)) in sources.iter().zip(cursors).enumerate() {
            lease.check()?;
            budget(start)?;
            let context_empty = context_anchor
                .as_ref()
                .is_some_and(|(tag, a)| source.tag > *tag && a.timestamp == 0);
            let window = if cursor.more && !context_empty {
                let key = crate::keyring::key_for(&keys, &source.salt)?;
                let mut selection = LiveConversationSelection {
                    username: username.to_string(),
                    before: cursor.before.clone(),
                    include_anchor: false,
                    newer_context: false,
                    query: query.clone(),
                };
                if let Some((tag, anchor)) =
                    context_anchor.as_ref().filter(|_| cursor.before.is_none())
                {
                    if source.tag == *tag {
                        selection.before = Some(anchor.clone());
                        selection.include_anchor = true;
                    } else {
                        // Tie order is timestamp, source tag, local ID. Exclude
                        // same-second rows sorting ahead of the chosen source.
                        let until = anchor
                            .timestamp
                            .checked_add(i64::from(source.tag < *tag))
                            .ok_or_else(stale)?;
                        let mut filter = LiveMessageQuery::default();
                        filter.until_timestamp = Some(until);
                        selection.query = Some(filter);
                    }
                }
                match self.provider.read_live_conversation(
                    account,
                    &source.database,
                    &key,
                    &source.fingerprint,
                    &selection,
                    &lease.cancel,
                ) {
                    Ok(window) => window,
                    Err(e)
                        if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE"
                            && cursor.before.is_none() =>
                    {
                        empty_window()
                    }
                    Err(e) => return Err(e),
                }
            } else {
                empty_window()
            };
            if query
                .as_ref()
                .is_some_and(|q| window.rows.iter().any(|row| !q.matches(row)))
            {
                return Err(AppError::new(
                    "LIVE_PROTOCOL_INVALID",
                    "读取结果不符合本次查找条件",
                ));
            }
            if let Some((tag, anchor)) = context_anchor.as_ref().filter(|_| focus.is_some()) {
                lease.check()?;
                budget(start)?;
                let selection = LiveConversationSelection {
                    username: username.to_string(),
                    before: (source.tag == *tag).then(|| anchor.clone()),
                    include_anchor: false,
                    newer_context: true,
                    query: if source.tag == *tag {
                        None
                    } else {
                        let mut filter = LiveMessageQuery::default();
                        filter.from_timestamp = Some(
                            anchor
                                .timestamp
                                .checked_add(i64::from(source.tag < *tag))
                                .ok_or_else(stale)?,
                        );
                        Some(filter)
                    },
                };
                let key = crate::keyring::key_for(&keys, &source.salt)?;
                let newer = match self.provider.read_live_conversation(
                    account,
                    &source.database,
                    &key,
                    &source.fingerprint,
                    &selection,
                    &lease.cancel,
                ) {
                    Ok(w) => w,
                    Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" && source.tag != *tag => {
                        empty_window()
                    }
                    Err(e) => return Err(e),
                };
                crate::live_conversation::validate_newer_context(&newer, &selection)?;
                newer_rows.extend(newer.rows.into_iter().map(|message| {
                    crate::live_multi_view::SourcedLiveMessage {
                        source: source.tag.clone(),
                        source_index: index + 1,
                        message,
                    }
                }));
            }
            merger.push(&source.tag, window, cursor)?;
        }
        lease.check()?;
        budget(start)?;
        if self.multi_conversation_binding(account, &sources, &username)? != binding {
            return Err(stale());
        }
        let view = merger.finish_with_context(&binding, newer_rows)?;
        let (messages, media, cards) = public_page(view.page(&binding, None, 60)?);
        if focus.as_ref().is_some_and(|f| {
            !messages.rows.iter().any(|row| {
                row.message.local_id == f.local_id && Some(&row.source) == f.source.as_ref()
            })
        }) {
            return Err(stale());
        }
        let mut slot = self
            .multi_conversation_result
            .lock()
            .map_err(|_| AppError::internal())?;
        lease.check()?;
        if self.multi_conversation_generation.load(Ordering::SeqCst) != generation {
            return Err(AppError::new("QUERY_CANCELED", "跨分片会话读取已取消"));
        }
        let output = MultiConversationPage {
            focus: focus.clone(),
            query: query.clone(),
            username: username.to_string(),
            messages,
            media,
            cards,
            source_scope: "selected_message_sources",
            sending_enabled: false,
        };
        *slot = Some(MultiConversationResult {
            context_ceiling: context_anchor,
            focus,
            query,
            account: account.into(),
            sources,
            username,
            view,
        });
        Ok(Some(output))
    }
}
