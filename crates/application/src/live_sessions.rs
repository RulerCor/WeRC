use crate::{Application, live_multi_view::MultiReadView, live_view::LiveReadView};
use serde::Serialize;
use werc_domain::{AppError, AppResult, live::*};

#[derive(Serialize)]
pub struct LiveSessionPage {
    pub search_query: Option<String>,
    pub rows: Vec<LiveSession>,
    pub next: Option<String>,
    pub expires_in_seconds: u64,
    pub mode: &'static str,
    pub complete_session_list: bool,
    pub cross_page_snapshot: bool,
}
pub(crate) enum ParentView<'a> {
    Single(&'a LiveReadView),
    Multi(&'a MultiReadView),
}
impl ParentView<'_> {
    pub(crate) fn check(&self, binding: &str, id: &str) -> AppResult<u64> {
        match self {
            Self::Single(v) => v.check_view(binding, id),
            Self::Multi(v) => v.check_view(binding, id),
        }
    }
    fn open(&self, binding: &str, id: &str, cursor: &str) -> AppResult<LiveSessionContinuation> {
        match self {
            Self::Single(v) => v.open_sessions(binding, id, cursor),
            Self::Multi(v) => v.open_sessions(binding, id, cursor),
        }
    }
    fn seal(&self, binding: &str, id: &str, value: &LiveSessionContinuation) -> AppResult<String> {
        match self {
            Self::Single(v) => v.seal_sessions(binding, id, value),
            Self::Multi(v) => v.seal_sessions(binding, id, value),
        }
    }
}
impl Application {
    // The multi-source parent still refers to one account. Its first authenticated
    // message source identifies that account's session cache; rows are not merged
    // from unrelated session databases. Every selected source remains pinned.
    pub(crate) fn with_live_source_parent<T>(
        &self,
        account: &str,
        view_id: &str,
        f: impl FnOnce(&str, &str, &str, ParentView<'_>) -> AppResult<T>,
    ) -> AppResult<T> {
        let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
        if let Some(r) = slot
            .as_ref()
            .filter(|r| r.account == account && r.view.matches_view(view_id))
        {
            let (binding, _, source) = self.live_read_binding(account, &r.database)?;
            if binding != r.binding {
                return Err(stale());
            }
            r.view.check_view(&binding, view_id)?;
            return f(&r.database, &binding, &source, ParentView::Single(&r.view));
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
        r.view.check_view(&binding, view_id)?;
        let first = r.sources.first().ok_or_else(stale)?;
        f(
            &first.database,
            &binding,
            &first.fingerprint,
            ParentView::Multi(&r.view),
        )
    }
    /// Only the session cache beside an authenticated message source set.
    /// Neither paths nor raw anchors are accepted from callers.
    pub fn live_sessions_local(
        &self,
        account: &str,
        view_id: &str,
        cursor: Option<&str>,
        request_id: &str,
    ) -> AppResult<LiveSessionPage> {
        self.live_sessions_search_local(account, view_id, cursor, None, request_id)
    }
    pub fn live_sessions_search_local(
        &self,
        account: &str,
        view_id: &str,
        cursor: Option<&str>,
        search_query: Option<&str>,
        request_id: &str,
    ) -> AppResult<LiveSessionPage> {
        let search_query = search_query.map(str::trim).filter(|q| !q.is_empty());
        if search_query.is_some_and(|q| !valid_session_query(q)) {
            return Err(AppError::new(
                "LIVE_QUERY_INVALID",
                "会话搜索文字最多256个UTF-8字节",
            ));
        }
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        lease.check()?;
        let (database, binding, source, after) =
            self.with_live_source_parent(account, view_id, |database, binding, source, view| {
                let after = cursor.map(|c| view.open(binding, view_id, c)).transpose()?;
                Ok((
                    zeroize::Zeroizing::new(database.to_owned()),
                    binding.to_owned(),
                    source.to_owned(),
                    after,
                ))
            })?;
        let query = after
            .as_ref()
            .and_then(|a| a.search_query.as_deref())
            .or(search_query);
        if after
            .as_ref()
            .is_some_and(|a| search_query.is_some_and(|q| a.search_query.as_deref() != Some(q)))
        {
            return Err(stale());
        }
        if query.is_some_and(|q| !valid_session_query(q)) {
            return Err(stale());
        }
        let (fingerprint, window) = self.provider.read_live_sessions_filtered(
            account,
            &database,
            &source,
            after.as_ref(),
            query,
            &lease.cancel,
        )?;
        lease.check()?;
        if fingerprint.len() != 64
            || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            || after
                .as_ref()
                .is_some_and(|a| a.source_fingerprint != fingerprint)
            || !window.valid(after.as_ref().map(|a| &a.anchor))
            || query.is_some_and(|q| window.rows.iter().any(|row| !row.matches_query(q)))
        {
            return Err(AppError::new(
                "LIVE_PROTOCOL_INVALID",
                "会话分页来源或结果无效",
            ));
        }
        let plain =
            zeroize::Zeroizing::new(serde_json::to_vec(&window).map_err(|_| AppError::internal())?);
        if plain.len() > LIVE_SESSION_MAX_BYTES {
            return Err(AppError::new(
                "LIVE_SESSION_WINDOW_LIMIT",
                "会话页超过读取上限",
            ));
        }
        self.with_live_source_parent(
            account,
            view_id,
            |current_database, current_binding, current_source, view| {
                if current_database != database.as_str()
                    || current_binding != binding
                    || current_source != source
                {
                    return Err(stale());
                }
                let expires_in_seconds = view.check(&binding, view_id)?;
                lease.check()?;
                let next = if window.has_more {
                    Some(view.seal(
                        &binding,
                        view_id,
                        &LiveSessionContinuation {
                            search_query: query.map(str::to_owned),
                            source_fingerprint: fingerprint,
                            anchor: window.rows.last().ok_or_else(stale)?.anchor.clone(),
                        },
                    )?)
                } else {
                    None
                };
                Ok(LiveSessionPage {
                    search_query: query.map(str::to_owned),
                    rows: window.rows,
                    next,
                    expires_in_seconds,
                    mode: "local_session_cache",
                    complete_session_list: false,
                    cross_page_snapshot: false,
                })
            },
        )
    }
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "在线来源窗口已变化或过期，请重新读取")
}
