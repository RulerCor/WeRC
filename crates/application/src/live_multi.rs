use crate::{
    Application,
    live_multi_view::{MAX_SOURCES, MultiLivePage, MultiReadView, SourceCursor, WindowMerger},
    live_view::LiveDirection,
};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
use werc_domain::{AppError, AppResult, live::LiveWindow};
use zeroize::Zeroizing;

#[derive(Clone)]
pub(crate) struct Source {
    pub(crate) database: Zeroizing<String>,
    pub(crate) binding: String,
    pub(crate) salt: [u8; 16],
    pub(crate) fingerprint: String,
    pub(crate) tag: String,
}
pub(crate) struct MultiResult {
    pub(crate) account: String,
    pub(crate) sources: Vec<Source>,
    pub(crate) view: MultiReadView,
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "跨分片来源或当前身份已变化，请重新读取")
}
pub(crate) fn budget(start: Instant) -> AppResult<()> {
    if start.elapsed() > Duration::from_secs(60) {
        Err(AppError::new(
            "LIVE_MULTI_TIMEOUT",
            "跨分片读取达到60秒上限，未发布部分结果",
        ))
    } else {
        Ok(())
    }
}
impl Application {
    pub(crate) fn multi_context(&self, account: &str, sources: &[Source]) -> AppResult<String> {
        for source in sources {
            if self.live_read_binding(account, &source.database)?.0 != source.binding {
                return Err(stale());
            }
        }
        self.send_digest(
            account,
            "live-multi-context-v1",
            &sources
                .iter()
                .map(|s| (&s.tag, &s.binding))
                .collect::<Vec<_>>(),
        )
    }
    pub fn prepare_live_filehelper_multi_local(
        &self,
        account: &str,
        databases: &[String],
        request_id: &str,
        confirmed: bool,
    ) -> AppResult<MultiLivePage> {
        validate_selection(databases, confirmed)?;
        let start = Instant::now();
        let lease = self.source_lease(request_id)?;
        self.prepare_multi_selected(account, databases, &lease, start)
    }
    pub fn prepare_live_filehelper_multi_with_keys_local(
        &self,
        account: &str,
        databases: &[String],
        pid: u32,
        request_id: &str,
        confirmed: bool,
    ) -> AppResult<MultiLivePage> {
        validate_selection(databases, confirmed)?;
        let start = Instant::now();
        let lease = self.source_lease(request_id)?;
        for database in databases {
            lease.check()?;
            budget(start)?;
            self.bind_live_source_local(account, database, true)?;
            let salt = self.provider.database_salt(database)?;
            let available = self
                .live_keys_local(account)
                .and_then(|keys| crate::keyring::key_for(&keys, &salt));
            match available {
                Ok(_) => (),
                Err(e) if matches!(e.code.as_str(), "KEY_REQUIRED" | "KEY_SOURCE_MISMATCH") => {
                    self.acquire_database_key_local(account, pid, database, true)?;
                }
                Err(e) => return Err(e),
            }
            lease.check()?;
            budget(start)?;
        }
        self.prepare_multi_selected(account, databases, &lease, start)
    }
    fn prepare_multi_selected(
        &self,
        account: &str,
        databases: &[String],
        lease: &crate::history::QueryLease<'_>,
        start: Instant,
    ) -> AppResult<MultiLivePage> {
        let _guard = self.history_guard.exclusive(|| lease.check())?;
        lease.check()?;
        let mut selected = databases.to_vec();
        selected.sort_by_key(|s| s.to_lowercase());
        let mut seen = BTreeSet::new();
        let mut sources = Vec::new();
        for database in selected {
            lease.check()?;
            budget(start)?;
            let (binding, salt, fingerprint) = self.live_read_binding(account, &database)?;
            if !seen.insert(fingerprint.clone()) {
                return Err(AppError::new(
                    "LIVE_SOURCE_DUPLICATE",
                    "所选分片包含同一来源文件",
                ));
            }
            self.provider.validate_live_source(account, &database)?;
            let tag =
                self.send_digest(account, "live-multi-source-v1", &(&binding, &fingerprint))?;
            sources.push(Source {
                database: Zeroizing::new(database),
                binding,
                salt,
                fingerprint,
                tag,
            });
        }
        lease.check()?;
        let cursors = sources
            .iter()
            .map(|_| SourceCursor {
                before: None,
                more: true,
            })
            .collect();
        self.read_multi_inner(account, sources, cursors, lease, start)
    }
    pub fn continue_live_filehelper_multi_local(
        &self,
        account: &str,
        cursor: &str,
        request_id: &str,
    ) -> AppResult<MultiLivePage> {
        let start = Instant::now();
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.exclusive(|| lease.check())?;
        lease.check()?;
        let (sources, cursors) = {
            let slot = self
                .multi_live_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let result = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let binding = self.multi_context(account, &result.sources)?;
            (
                result.sources.clone(),
                result.view.continuation(&binding, cursor)?,
            )
        };
        self.read_multi_inner(account, sources, cursors, &lease, start)
    }
    fn read_multi_inner(
        &self,
        account: &str,
        sources: Vec<Source>,
        cursors: Vec<SourceCursor>,
        lease: &crate::history::QueryLease<'_>,
        start: Instant,
    ) -> AppResult<MultiLivePage> {
        lease.check()?;
        budget(start)?;
        if cursors.len() != sources.len() {
            return Err(stale());
        }
        let binding = self.multi_context(account, &sources)?;
        let keys = self.live_keys_local(account)?;
        let mut merger = WindowMerger::new();
        for (source, cursor) in sources.iter().zip(cursors) {
            lease.check()?;
            budget(start)?;
            let window = if cursor.more {
                let key = crate::keyring::key_for(&keys, &source.salt)?;
                match self.provider.read_live_filehelper(
                    account,
                    &source.database,
                    &key,
                    &source.fingerprint,
                    cursor.before.as_ref(),
                    &lease.cancel,
                ) {
                    Ok(window) => window,
                    // Exact filehelper table absent is a valid empty result only
                    // at the first read. Losing an anchored table is stale.
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
            merger.push(&source.tag, window, cursor)?;
        }
        lease.check()?;
        budget(start)?;
        if self.multi_context(account, &sources)? != binding {
            return Err(stale());
        }
        for source in &sources {
            self.enroll_backfill_source(account, &source.database)?;
        }
        let view = merger.finish(&binding)?;
        let page = view.page(&binding, None, 60)?;
        lease.check()?;
        budget(start)?;
        self.live_result
            .lock()
            .map_err(|_| AppError::internal())?
            .take();
        *self
            .multi_live_result
            .lock()
            .map_err(|_| AppError::internal())? = Some(MultiResult {
            account: account.into(),
            sources,
            view,
        });
        Ok(page)
    }
    pub fn live_filehelper_multi_page_local(
        &self,
        account: &str,
        cursor: Option<(&str, LiveDirection)>,
        limit: usize,
    ) -> AppResult<MultiLivePage> {
        let mut slot = self
            .multi_live_result
            .lock()
            .map_err(|_| AppError::internal())?;
        let result = slot.as_ref().ok_or_else(stale)?;
        if result.account != account || result.view.expired() {
            slot.take();
            return Err(stale());
        }
        let binding = match self.multi_context(account, &result.sources) {
            Ok(b) => b,
            Err(e) => {
                slot.take();
                return Err(e);
            }
        };
        result.view.page(&binding, cursor, limit)
    }
}
pub(crate) fn empty_window() -> LiveWindow {
    LiveWindow {
        schema_version: 1,
        rows: vec![],
        has_older: false,
        read_only: true,
        readonly_shm: true,
        read_mode: "bounded_transaction_window".into(),
    }
}

fn validate_selection(databases: &[String], confirmed: bool) -> AppResult<()> {
    if !confirmed {
        return Err(AppError::new(
            "CONSENT_REQUIRED",
            "需要确认只读合并所选消息分片",
        ));
    }
    if !(1..=MAX_SOURCES).contains(&databases.len()) {
        return Err(AppError::new(
            "LIVE_SOURCE_LIMIT",
            "每次需要1至16个明确来源",
        ));
    }
    Ok(())
}
