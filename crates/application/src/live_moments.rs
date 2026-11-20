//! Authenticated local timeline browsing and complete selected-cache scans.
use crate::{Application, history::QueryLease, live_search::SearchTicket};
use serde::Serialize;
use werc_domain::{AppError, AppResult, moments::*};
mod tools;
pub(crate) use tools::details_text;
pub use tools::*;

#[derive(Serialize)]
pub struct LiveMomentPage {
    pub rows: Vec<LiveMoment>,
    pub next: Option<String>,
    pub query: MomentQuery,
    pub scanned: usize,
    pub expires_in_seconds: u64,
    pub mode: &'static str,
    pub complete_timeline: bool,
    pub cross_page_snapshot: bool,
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "朋友圈来源或登录状态已变化，请重新读取")
}
impl Application {
    pub fn live_moments_local(
        &self,
        account: &str,
        view: &str,
        cursor: Option<&str>,
        request: &str,
    ) -> AppResult<LiveMomentPage> {
        self.query_live_moments_local(account, view, None, cursor, request)
    }
    pub fn query_live_moments_local(
        &self,
        account: &str,
        view: &str,
        query: Option<MomentQuery>,
        cursor: Option<&str>,
        request: &str,
    ) -> AppResult<LiveMomentPage> {
        let lease = self.source_lease(request)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        self.moments_page(account, view, query, cursor, &lease, LIVE_MOMENT_MAX_ROWS)
    }
    fn moments_page(
        &self,
        account: &str,
        view: &str,
        query: Option<MomentQuery>,
        cursor: Option<&str>,
        lease: &QueryLease<'_>,
        batch_size: usize,
    ) -> AppResult<LiveMomentPage> {
        if query.is_some() && cursor.is_some() {
            return Err(AppError::new("INVALID_ARGUMENT", "继续读取只接受游标"));
        }
        let (binding, source, mut after, query) =
            self.with_search_parent(account, view, |binding, parent, sources, _| {
                let (after, query) = if let Some(cursor) = cursor {
                    match parent.open_search(binding, view, cursor)? {
                        SearchTicket::Moments { after, query } => (Some(after), query),
                        _ => return Err(stale()),
                    }
                } else {
                    (None, query.unwrap_or_default())
                };
                if !query.valid() {
                    return Err(AppError::new("INVALID_ARGUMENT", "朋友圈筛选条件无效"));
                }
                Ok((
                    binding.to_owned(),
                    sources.first().ok_or_else(stale)?.clone(),
                    after,
                    query,
                ))
            })?;
        let mut rows = Vec::new();
        let mut scanned = 0;
        // A sparse query returns an authenticated continuation after six source pages.
        for _ in 0..6 {
            lease.check()?;
            let (fingerprint, window) = self.provider.read_live_moments_batch(
                account,
                &source.database,
                &source.fingerprint,
                after.as_ref(),
                batch_size,
                &lease.cancel,
            )?;
            lease.check()?;
            if fingerprint.len() != 64
                || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                || after
                    .as_ref()
                    .is_some_and(|a| a.source_fingerprint != fingerprint)
                || !(window.valid(after.as_ref().map(|a| &a.anchor))
                    || window.valid_with_limit(after.as_ref().map(|a| &a.anchor), batch_size))
            {
                return Err(AppError::new(
                    "LIVE_PROTOCOL_INVALID",
                    "朋友圈分页来源或结果无效",
                ));
            }
            let encoded = zeroize::Zeroizing::new(
                serde_json::to_vec(&window).map_err(|_| AppError::internal())?,
            );
            if encoded.len()
                > if batch_size == LIVE_MOMENT_MAX_ROWS {
                    LIVE_MOMENT_MAX_BYTES
                } else {
                    LIVE_MOMENT_SCAN_BYTES
                }
            {
                return Err(AppError::new(
                    "LIVE_MOMENT_WINDOW_LIMIT",
                    "朋友圈页超过读取上限",
                ));
            }
            scanned += window.rows.len();
            after = if window.has_more {
                Some(LiveMomentContinuation {
                    source_fingerprint: fingerprint,
                    anchor: window.rows.last().ok_or_else(stale)?.anchor.clone(),
                })
            } else {
                None
            };
            rows.extend(window.rows.into_iter().filter(|r| query.matches(r)));
            if !rows.is_empty() || after.is_none() {
                break;
            }
        }
        self.with_search_parent(account, view, |current, parent, sources, _| {
            if current != binding
                || sources
                    .first()
                    .is_none_or(|s| s.fingerprint != source.fingerprint)
            {
                return Err(stale());
            }
            let expires = parent.check(current, view)?;
            let next = after
                .map(|after| {
                    parent.seal_search(
                        current,
                        view,
                        &SearchTicket::Moments {
                            after,
                            query: query.clone(),
                        },
                    )
                })
                .transpose()?;
            lease.check()?;
            self.backfill_cached_moment_rows(account, &rows)?;
            Ok(LiveMomentPage {
                rows,
                next,
                query,
                scanned,
                expires_in_seconds: expires,
                mode: "local_moment_cache",
                complete_timeline: false,
                cross_page_snapshot: false,
            })
        })
    }
    pub(crate) fn scan_moments(
        &self,
        account: &str,
        view: &str,
        query: &MomentQuery,
        lease: &QueryLease<'_>,
        mut consume: impl FnMut(LiveMoment) -> AppResult<()>,
    ) -> AppResult<usize> {
        let mut cursor = None;
        let mut scanned = 0;
        loop {
            let page = self.moments_page(
                account,
                view,
                if cursor.is_none() {
                    Some(query.clone())
                } else {
                    None
                },
                cursor.as_deref(),
                lease,
                LIVE_MOMENT_SCAN_ROWS,
            )?;
            scanned += page.scanned;
            if scanned > 50_000 {
                return Err(AppError::new(
                    "MOMENT_SCAN_LIMIT",
                    "朋友圈缓存超过五万条，请分批读取",
                ));
            }
            for row in page.rows {
                lease.check()?;
                consume(row)?;
            }
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        lease.check()?;
        Ok(scanned)
    }
}
