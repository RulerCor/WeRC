//! Cross-conversation search over the user's authenticated source selection.
use crate::{Application, live_multi::Source, live_sessions::ParentView};
use serde::{Deserialize, Serialize};
use werc_domain::{AppError, AppResult, live::*};
use zeroize::{Zeroize, Zeroizing};

#[derive(Serialize, Deserialize)]
#[serde(tag = "purpose", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SearchTicket {
    SpecialRecords {
        fingerprint: String,
        selection: werc_domain::special_records::RecordSelection,
    },
    Moments {
        after: werc_domain::moments::LiveMomentContinuation,
        query: werc_domain::moments::MomentQuery,
    },
    Favorites {
        fingerprint: String,
        selection: werc_domain::favorites::FavoriteSelection,
    },
    FavoriteItem {
        fingerprint: String,
        anchor: werc_domain::favorites::FavoriteAnchor,
        digest: String,
    },
    Scan {
        source_index: usize,
        selection: LiveSearchSelection,
    },
    Hit {
        username: String,
        anchor: LiveAnchor,
        source: Option<String>,
    },
}
#[derive(Serialize)]
pub struct SearchRow {
    pub username: String,
    pub message: LiveMessage,
    pub source_index: usize,
    pub result_token: String,
}
#[derive(Serialize)]
pub struct SearchPage {
    pub rows: Vec<SearchRow>,
    pub query: LiveMessageQuery,
    pub next: Option<String>,
    pub scanned_conversations: usize,
    pub source_index: usize,
    pub source_count: usize,
    pub expires_in_seconds: u64,
    pub source_scope: &'static str,
    pub cross_page_snapshot: bool,
}
fn stale() -> AppError {
    AppError::new(
        "CURSOR_STALE",
        "搜索来源或登录身份已变化，请重新准备消息来源",
    )
}
impl ParentView<'_> {
    pub(crate) fn seal_search(
        &self,
        binding: &str,
        id: &str,
        ticket: &SearchTicket,
    ) -> AppResult<String> {
        match self {
            Self::Single(v) => v.seal_search(binding, id, ticket),
            Self::Multi(v) => v.seal_search(binding, id, ticket),
        }
    }
    pub(crate) fn open_search(
        &self,
        binding: &str,
        id: &str,
        ticket: &str,
    ) -> AppResult<SearchTicket> {
        match self {
            Self::Single(v) => v.open_search(binding, id, ticket),
            Self::Multi(v) => v.open_search(binding, id, ticket),
        }
    }
}
impl Application {
    pub(crate) fn with_search_parent<T>(
        &self,
        account: &str,
        id: &str,
        f: impl FnOnce(&str, ParentView<'_>, &[Source], bool) -> AppResult<T>,
    ) -> AppResult<T> {
        let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
        if let Some(r) = slot
            .as_ref()
            .filter(|r| r.account == account && r.view.matches_view(id))
        {
            let (binding, salt, fingerprint) = self.live_read_binding(account, &r.database)?;
            if binding != r.binding {
                return Err(stale());
            }
            r.view.check_view(&binding, id)?;
            let sources = [Source {
                database: r.database.clone(),
                binding: binding.clone(),
                salt,
                fingerprint,
                tag: String::new(),
            }];
            return f(&binding, ParentView::Single(&r.view), &sources, false);
        }
        drop(slot);
        let slot = self
            .multi_live_result
            .lock()
            .map_err(|_| AppError::internal())?;
        let r = slot
            .as_ref()
            .filter(|r| r.account == account)
            .ok_or_else(stale)?;
        let binding = self.multi_context(account, &r.sources)?;
        r.view.check_view(&binding, id)?;
        f(&binding, ParentView::Multi(&r.view), &r.sources, true)
    }
    pub fn search_live_messages_local(
        &self,
        account: &str,
        view_id: &str,
        query: Option<LiveMessageQuery>,
        cursor: Option<&str>,
        request_id: &str,
    ) -> AppResult<SearchPage> {
        if cursor.is_some() == query.is_some() || query.as_ref().is_some_and(|q| !q.valid()) {
            return Err(AppError::new(
                "LIVE_QUERY_INVALID",
                "首次搜索需要条件；继续搜索请只传续读游标",
            ));
        }
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        lease.check()?;
        let (binding, sources, multi, index, selection) =
            self.with_search_parent(account, view_id, |binding, parent, sources, multi| {
                let (index, selection) = if let Some(cursor) = cursor {
                    match parent.open_search(binding, view_id, cursor)? {
                        SearchTicket::Scan {
                            source_index,
                            selection,
                        } => (source_index, selection),
                        _ => return Err(stale()),
                    }
                } else {
                    (
                        0,
                        LiveSearchSelection {
                            query: query.ok_or_else(stale)?,
                            after: None,
                        },
                    )
                };
                if index >= sources.len() || !selection.valid() {
                    return Err(stale());
                }
                Ok((
                    binding.to_owned(),
                    sources.to_vec(),
                    multi,
                    index,
                    selection,
                ))
            })?;
        let source = &sources[index];
        let keys = self.live_keys_local(account)?;
        let key = crate::keyring::key_for(&keys, &source.salt)?;
        let window = self.provider.search_live_messages(
            account,
            &source.database,
            &key,
            &source.fingerprint,
            &selection,
            &lease.cancel,
        )?;
        lease.check()?;
        if !window.valid(&selection)
            || Zeroizing::new(serde_json::to_vec(&window).map_err(|_| AppError::internal())?).len()
                > LIVE_WINDOW_MAX_BYTES
        {
            return Err(AppError::new(
                "LIVE_PROTOCOL_INVALID",
                "跨会话搜索结果或续读位置无效",
            ));
        }
        self.with_search_parent(
            account,
            view_id,
            |current, parent, current_sources, current_multi| {
                if current != binding
                    || multi != current_multi
                    || current_sources.len() != sources.len()
                {
                    return Err(stale());
                }
                let expires = parent.check(current, view_id)?;
                let next = if let Some(next) = window.next {
                    Some(SearchTicket::Scan {
                        source_index: index,
                        selection: LiveSearchSelection {
                            query: selection.query.clone(),
                            after: Some(next),
                        },
                    })
                } else if index + 1 < sources.len() {
                    Some(SearchTicket::Scan {
                        source_index: index + 1,
                        selection: LiveSearchSelection {
                            query: selection.query.clone(),
                            after: None,
                        },
                    })
                } else {
                    None
                };
                let next = next
                    .as_ref()
                    .map(|t| parent.seal_search(current, view_id, t))
                    .transpose()?;
                let mut rows = Vec::with_capacity(window.rows.len());
                for hit in window.rows {
                    let token = parent.seal_search(
                        current,
                        view_id,
                        &SearchTicket::Hit {
                            username: hit.username.clone(),
                            anchor: LiveAnchor {
                                timestamp: hit.message.timestamp,
                                local_id: hit.message.local_id.clone(),
                                server_id: hit.message.server_id.clone(),
                            },
                            source: multi.then(|| source.tag.clone()),
                        },
                    )?;
                    let mut message = hit.message.clone();
                    message.metadata.zeroize();
                    rows.push(SearchRow {
                        username: hit.username.clone(),
                        message,
                        source_index: index + 1,
                        result_token: token,
                    });
                }
                lease.check()?;
                Ok(SearchPage {
                    rows,
                    query: selection.query.clone(),
                    next,
                    scanned_conversations: window.scanned_conversations,
                    source_index: index + 1,
                    source_count: sources.len(),
                    expires_in_seconds: expires,
                    source_scope: "selected_message_sources",
                    cross_page_snapshot: false,
                })
            },
        )
    }
    pub(crate) fn search_hit(
        &self,
        account: &str,
        parent_id: &str,
        token: &str,
        multi: bool,
    ) -> AppResult<(String, LiveAnchor, Option<String>)> {
        self.with_search_parent(account, parent_id, |binding, parent, sources, is_multi| {
            if multi != is_multi {
                return Err(stale());
            }
            match parent.open_search(binding, parent_id, token)? {
                SearchTicket::Hit {
                    username,
                    anchor,
                    source,
                } if valid_live_username(&username)
                    && anchor.key().is_some()
                    && (!multi && source.is_none()
                        || multi && sources.iter().any(|s| Some(&s.tag) == source.as_ref())) =>
                {
                    Ok((username, anchor, source))
                }
                _ => Err(stale()),
            }
        })
    }
}
