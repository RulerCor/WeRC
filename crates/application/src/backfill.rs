//! Durable source recovery, independent of the native hook's acknowledgement cursor.
//! Sources are enrolled after a successful authenticated read. Each step commits
//! one bounded source page and its continuation together; reconnect resumes it.
use crate::Application;
use serde::{Deserialize, Serialize};
use werc_domain::{AppError, AppResult, library::*, live::*};
use zeroize::Zeroize;

const STATE: &str = "automatic-source-recovery-v1";
const INTERVAL: i64 = 30;
const OVERLAP: i64 = 120;
const RESCAN: i64 = 86400;

#[derive(Serialize)]
#[serde(untagged)]
pub enum AutomaticLivePage {
    Single(crate::live_view::LivePage),
    Multiple(crate::live_multi_view::MultiLivePage),
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct BoundSource {
    pub database: String,
    pub salt: [u8; 16],
    pub fingerprint: String,
}
#[derive(Default)]
pub(crate) struct DiscoveryState {
    account: String,
    epoch: String,
    refresh_at: i64,
    process_id: u32,
    pending: std::collections::VecDeque<String>,
    error: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Source {
    database: String,
    salt: [u8; 16],
    fingerprint: String,
    completed_until: Option<i64>,
    full_scan_at: Option<i64>,
    until: Option<i64>,
    from: Option<i64>,
    after: Option<LiveSearchPosition>,
    error: Option<String>,
    retry_at: i64,
}
#[derive(Serialize, Deserialize)]
struct State {
    login: String,
    sources: Vec<Source>,
    next_source: usize,
    pages: u64,
    rows: u64,
}
#[derive(Serialize)]
pub struct BackfillStatus {
    pub source_count: usize,
    pub pending_sources: usize,
    pub pages: u64,
    pub rows: u64,
    pub observed_through: Option<i64>,
    pub errors: Vec<String>,
    pub historical_complete: bool,
}
#[derive(Serialize, Deserialize)]
pub struct BackfillRow {
    /// Stable within this account, source file and local row, including reconnects.
    pub id: String,
    pub content_digest: String,
    pub username: String,
    pub message: LiveMessage,
}
#[derive(Serialize, Deserialize)]
pub struct BackfillBatch {
    pub position: u64,
    pub source_id: String,
    pub observed_at: String,
    pub from: Option<i64>,
    pub until: i64,
    pub rows: Vec<BackfillRow>,
}
#[derive(Serialize)]
pub struct BackfillPage {
    pub data: Vec<BackfillBatch>,
    pub next: u64,
    pub has_more: bool,
    pub status: BackfillStatus,
}
fn status(state: Option<&State>) -> BackfillStatus {
    let Some(s) = state else {
        return BackfillStatus {
            source_count: 0,
            pending_sources: 0,
            pages: 0,
            rows: 0,
            observed_through: None,
            errors: vec![],
            historical_complete: false,
        };
    };
    BackfillStatus {
        source_count: s.sources.len(),
        pending_sources: s
            .sources
            .iter()
            .filter(|v| v.until.is_some() || v.completed_until.is_none())
            .count(),
        pages: s.pages,
        rows: s.rows,
        observed_through: s
            .sources
            .iter()
            .map(|v| v.completed_until)
            .collect::<Option<Vec<_>>>()
            .and_then(|v| v.into_iter().min()),
        errors: s
            .sources
            .iter()
            .filter_map(|v| v.error.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        historical_complete: false,
    }
}
impl Application {
    /// Recovery and foreground views use the same exact account-root choice.
    pub(crate) fn selected_automatic_sources_local(
        &self,
        account: &str,
    ) -> AppResult<crate::ports::AutomaticMessageSources> {
        let context = self.checked_send_context(account, "filehelper")?;
        let mut found = self.provider.automatic_message_sources(account)?;
        if found.databases.len() > 512 || found.process_id == 0 {
            return Err(AppError::new("LIVE_PROTOCOL_INVALID", "自动来源结果无效"));
        }
        let mut roots = std::collections::BTreeMap::<String, Vec<String>>::new();
        let mut seen = std::collections::BTreeSet::new();
        for database in found.databases {
            self.provider.validate_live_source(account, &database)?;
            let canonical = std::fs::canonicalize(&database)
                .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "自动消息来源无法访问"))?;
            if !seen.insert(canonical.to_string_lossy().to_lowercase()) {
                continue;
            }
            let root = crate::onboarding::source_root(&canonical)?;
            roots.entry(root).or_default().push(database);
        }
        let mut owned = Vec::new();
        for root in roots.keys() {
            use sha2::{Digest, Sha256};
            let fingerprint = hex::encode(Sha256::digest(
                format!("WeRC/setup-source/v1\0{root}").as_bytes(),
            ));
            if self.repository.source_owner(&fingerprint)?.as_deref() == Some(account) {
                owned.push(root.clone());
            }
        }
        let selected = if owned.len() == 1 {
            owned.pop()
        } else if owned.is_empty() && roots.len() == 1 {
            roots.keys().next().cloned()
        } else {
            None
        };
        if !roots.is_empty() && selected.is_none() {
            return Err(AppError::new(
                "SOURCE_SELECTION_REQUIRED",
                "发现多个当前账号目录，请在高级来源中选择一次",
            ));
        }
        found.databases = selected
            .and_then(|root| roots.remove(&root))
            .unwrap_or_default();
        found
            .databases
            .sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        if self.checked_send_context(account, "filehelper")? != context {
            return Err(AppError::new("CONNECTION_CHANGED", "发现来源期间登录变化"));
        }
        Ok(found)
    }
    pub fn prepare_automatic_live_local(
        &self,
        account: &str,
        request_id: &str,
        single: bool,
    ) -> AppResult<AutomaticLivePage> {
        let found = self.selected_automatic_sources_local(account)?;
        if found.databases.is_empty() {
            return Err(AppError::new(
                "SOURCE_NOT_FOUND",
                "当前登录账号没有可在线读取的消息来源",
            ));
        }
        if single || found.databases.len() == 1 {
            // Numeric order is required: message_10 follows message_9.
            let database = found
                .databases
                .iter()
                .max_by_key(|s| {
                    std::path::Path::new(s)
                        .file_stem()
                        .and_then(|v| v.to_str())
                        .and_then(|v| v.strip_prefix("message_"))
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(0)
                })
                .ok_or_else(AppError::internal)?;
            self.bind_live_source_local(account, database, true)?;
            let page = match self.prepare_live_filehelper_local(account, database, request_id, true)
            {
                Err(e) if matches!(e.code.as_str(), "KEY_REQUIRED" | "KEY_SOURCE_MISMATCH") => {
                    self.acquire_database_key_local(account, found.process_id, database, true)?;
                    self.prepare_live_filehelper_local(account, database, request_id, true)?
                }
                result => result?,
            };
            Ok(AutomaticLivePage::Single(page))
        } else {
            if found.databases.len() > crate::live_multi_view::MAX_SOURCES {
                return Err(AppError::new(
                    "LIVE_SOURCE_LIMIT",
                    "当前账号超过16个消息分片，请在高级来源中明确选择读取范围",
                ));
            }
            Ok(AutomaticLivePage::Multiple(
                self.prepare_live_filehelper_multi_with_keys_local(
                    account,
                    &found.databases,
                    found.process_id,
                    request_id,
                    true,
                )?,
            ))
        }
    }
    fn backfill_state(&self, account: &str) -> AppResult<Option<(DocumentHeader, State)>> {
        match self.open_document(account, STATE, "backfill_state") {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.code == "NOT_FOUND" => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub(crate) fn enrolled_message_sources(
        &self,
        account: &str,
        login: &str,
    ) -> AppResult<Vec<BoundSource>> {
        let Some((_, state)) = self.backfill_state(account)? else {
            return Ok(vec![]);
        };
        if state.login != login {
            return Err(AppError::new("CONNECTION_CHANGED", "回执来源登录身份变化"));
        }
        Ok(state
            .sources
            .into_iter()
            .map(|source| BoundSource {
                database: source.database,
                salt: source.salt,
                fingerprint: source.fingerprint,
            })
            .collect())
    }
    pub fn backfill_status_local(&self, account: &str) -> AppResult<BackfillStatus> {
        let mut result = status(self.backfill_state(account)?.as_ref().map(|(_, s)| s));
        if let Ok(discovery) = self.backfill_discovery.try_lock() {
            if discovery.account == account {
                if let Some(error) = &discovery.error {
                    if !result.errors.contains(error) {
                        result.errors.push(error.clone());
                    }
                }
            }
        }
        Ok(result)
    }
    /// Discover once per minute, preparing at most one new shard per step.
    /// Uses the already connected login; ambiguous roots require the existing source picker.
    pub fn discover_backfill_sources_local(&self, account: &str) -> AppResult<()> {
        let _history = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源读取正在执行"))?;
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let mut discovery = self
            .backfill_discovery
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "来源发现正在进行"))?;
        if discovery.account != account || discovery.epoch != context.epoch {
            *discovery = DiscoveryState {
                account: account.into(),
                epoch: context.epoch.clone(),
                ..Default::default()
            };
        }
        let now = chrono::Utc::now().timestamp();
        if discovery.pending.is_empty() && now < discovery.refresh_at {
            return Ok(());
        }
        let result = (|| {
            if discovery.pending.is_empty() {
                discovery.refresh_at = now + 60;
                let found = self.selected_automatic_sources_local(account)?;
                discovery.process_id = found.process_id;
                discovery.pending = found.databases.into();
            }
            while let Some(database) = discovery.pending.pop_front() {
                self.provider.validate_live_source(account, &database)?;
                let salt = self.provider.database_salt(&database)?;
                let fingerprint = self.provider.live_source_fingerprint(&database)?;
                let enrolled = self
                    .backfill_state(account)?
                    .as_ref()
                    .is_some_and(|(_, state)| {
                        state.login == context.sender_id
                            && state.sources.iter().any(|s| {
                                s.fingerprint == fingerprint
                                    && s.salt == salt
                                    && s.database == database
                            })
                    });
                self.bind_live_source_local(account, &database, true)?;
                let keys = self.live_keys_local(account);
                let has_key = match &keys {
                    Ok(keys) => crate::keyring::key_for(keys, &salt).is_ok(),
                    Err(e) if matches!(e.code.as_str(), "KEY_REQUIRED" | "KEY_SOURCE_MISMATCH") => {
                        false
                    }
                    Err(e) => return Err(e.clone()),
                };
                if enrolled && has_key {
                    continue;
                }
                if !has_key {
                    self.acquire_database_key_guarded(account, discovery.process_id, &database)?;
                }
                if self.checked_send_context(account, "filehelper")? != (context.clone(), revision)
                {
                    return Err(AppError::new("CONNECTION_CHANGED", "准备来源期间登录变化"));
                }
                self.enroll_backfill_source(account, &database)?;
                break;
            }
            Ok(())
        })();
        match &result {
            Ok(()) => discovery.error = None,
            Err(e) => {
                discovery.error = Some(e.code.clone());
                discovery.refresh_at = now + 60;
                discovery.pending.clear();
            }
        }
        result
    }
    pub(crate) fn enroll_backfill_source(&self, account: &str, database: &str) -> AppResult<()> {
        let _guard = self
            .backfill_guard
            .lock()
            .map_err(|_| AppError::internal())?;
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let (_, salt, fingerprint) = self.live_read_binding(account, database)?;
        let key_revision = self.repository.key_revision(account)?;
        let (old_revision, mut state) = match self.backfill_state(account)? {
            Some((h, s)) => (h.revision, s),
            None => (
                0,
                State {
                    login: context.sender_id.clone(),
                    sources: vec![],
                    next_source: 0,
                    pages: 0,
                    rows: 0,
                },
            ),
        };
        if state.login != context.sender_id {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "补读来源与当前登录者不同",
            ));
        }
        // Existing canonical file identity keeps its checkpoint across reconnection.
        if let Some(source) = state
            .sources
            .iter_mut()
            .find(|s| s.fingerprint == fingerprint && s.salt == salt)
        {
            if source.database == database {
                return Ok(());
            }
            source.database = database.into();
        } else {
            if state.sources.len() >= 512 {
                return Err(AppError::new(
                    "SOURCE_LIMIT_EXCEEDED",
                    "自动补读来源超过512个",
                ));
            }
            state.sources.push(Source {
                database: database.into(),
                salt,
                fingerprint,
                completed_until: None,
                full_scan_at: None,
                until: None,
                from: None,
                after: None,
                error: None,
                retry_at: 0,
            });
        }
        let document =
            self.seal_document(account, STATE, "backfill_state", old_revision + 1, &state)?;
        self.repository.library_save(
            &[document],
            Some(&DocumentConnection {
                revision,
                key_revision,
            }),
        )
    }
    /// Run on the application's source worker, never in the native receive callback.
    /// Busy foreground reads leave this checkpoint untouched and have priority.
    pub fn step_backfill_local(&self, account: &str) -> AppResult<BackfillStatus> {
        let _history = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源读取正在执行"))?;
        let _guard = self
            .backfill_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "补读正在执行"))?;
        let Some((header, mut state)) = self.backfill_state(account)? else {
            return Ok(status(None));
        };
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        if context.sender_id != state.login {
            return Err(AppError::new("CONNECTION_CHANGED", "补读账号已变化"));
        }
        let key_revision = self.repository.key_revision(account)?;
        let now = chrono::Utc::now().timestamp();
        let count = state.sources.len();
        let Some(index) = (0..count)
            .map(|offset| (state.next_source + offset) % count)
            .find(|&i| {
                let s = &state.sources[i];
                s.retry_at <= now
                    && (s.until.is_some() || s.completed_until.is_none_or(|t| now >= t + INTERVAL))
            })
        else {
            return Ok(status(Some(&state)));
        };
        let source = &mut state.sources[index];
        if source.until.is_none() {
            source.until = Some(now);
            // Daily full scans recover late arrivals carrying old timestamps.
            source.from = if source.full_scan_at.is_none_or(|t| now >= t + RESCAN) {
                None
            } else {
                source
                    .completed_until
                    .map(|t| t.saturating_sub(OVERLAP).max(0))
            };
            source.after = None;
        }
        let until = source.until.ok_or_else(AppError::internal)?;
        let mut query = LiveMessageQuery::default();
        query.from_timestamp = source.from;
        query.until_timestamp = Some(until);
        let selection = LiveSearchSelection {
            query,
            after: source.after.clone(),
        };
        let read = (|| {
            let (binding, salt, fingerprint) = self.live_read_binding(account, &source.database)?;
            if salt != source.salt || fingerprint != source.fingerprint {
                return Err(AppError::new(
                    "LIVE_SOURCE_REPLACED",
                    "补读来源被替换，请重新读取消息来源",
                ));
            }
            let keys = self.live_keys_local(account)?;
            let key = crate::keyring::key_for(&keys, &salt)?;
            let lease = self.browse_lease("automatic-source-recovery")?;
            let window = self.provider.search_live_messages(
                account,
                &source.database,
                &key,
                &fingerprint,
                &selection,
                &lease.cancel,
            )?;
            lease.check()?;
            if !window.valid(&selection) {
                return Err(AppError::new(
                    "LIVE_PROTOCOL_INVALID",
                    "补读结果或续页位置无效",
                ));
            }
            if self.live_read_binding(account, &source.database)?.0 != binding {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "读取期间来源或身份变化",
                ));
            }
            Ok(window)
        })();
        state.next_source = (index + 1) % count;
        let mut documents = Vec::new();
        match read {
            Ok(window) => {
                let mut rows = Vec::with_capacity(window.rows.len());
                let source_id = self.send_digest(
                    account,
                    "backfill-source-v1",
                    &(&state.login, &source.fingerprint, source.salt),
                )?;
                for hit in &window.rows {
                    let id = self.send_digest(
                        account,
                        "backfill-row-v1",
                        &(&source_id, &hit.username, &hit.message.local_id),
                    )?;
                    let content_digest =
                        self.send_digest(account, "backfill-content-v1", &hit.message)?;
                    rows.push(BackfillRow {
                        id,
                        content_digest,
                        username: hit.username.clone(),
                        message: hit.message.clone(),
                    });
                }
                if !rows.is_empty() {
                    state.pages = state.pages.checked_add(1).ok_or_else(AppError::internal)?;
                    state.rows = state
                        .rows
                        .checked_add(rows.len() as u64)
                        .ok_or_else(AppError::internal)?;
                    let batch = BackfillBatch {
                        position: state.pages,
                        source_id,
                        observed_at: chrono::Utc::now().to_rfc3339(),
                        from: source.from,
                        until,
                        rows,
                    };
                    documents.push(self.seal_document(
                        account,
                        &format!("{STATE}:{}", state.pages),
                        "backfill_page",
                        1,
                        &batch,
                    )?);
                }
                source.after = window.next;
                source.error = None;
                source.retry_at = 0;
                if source.after.is_none() {
                    source.completed_until = Some(until);
                    if source.from.is_none() {
                        source.full_scan_at = Some(until);
                    }
                    source.until = None;
                }
            }
            Err(e) => {
                if matches!(e.code.as_str(), "BUSY" | "CANCELED" | "CONNECTION_CHANGED") {
                    return Err(e);
                }
                source.error = Some(e.code);
                source.retry_at = now + INTERVAL;
            }
        }
        let result = status(Some(&state));
        documents.push(self.seal_document(
            account,
            STATE,
            "backfill_state",
            header.revision + 1,
            &state,
        )?);
        self.repository.library_save(
            &documents,
            Some(&DocumentConnection {
                revision,
                key_revision,
            }),
        )?;
        Ok(result)
    }
    pub fn backfill_page_local(
        &self,
        account: &str,
        after: u64,
        limit: u32,
    ) -> AppResult<BackfillPage> {
        if !(1..=10).contains(&limit) {
            return Err(AppError::new("INVALID_ARGUMENT", "补读每次读取1至10页"));
        }
        let current = self.backfill_status_local(account)?;
        if after > current.pages {
            return Err(AppError::new("CURSOR_STALE", "补读位置超出已保存范围"));
        }
        let end = after.saturating_add(u64::from(limit)).min(current.pages);
        let mut data = Vec::new();
        for position in after + 1..=end {
            let (_, mut batch): (_, BackfillBatch) =
                self.open_document(account, &format!("{STATE}:{position}"), "backfill_page")?;
            if batch.position != position {
                return Err(AppError::new("DOCUMENT_CORRUPTED", "补读页位置无效"));
            }
            // Raw XML may contain CDN credentials; public recovery feeds expose display data.
            for row in &mut batch.rows {
                row.message.metadata.zeroize();
                row.message.metadata = None;
            }
            data.push(batch);
        }
        Ok(BackfillPage {
            data,
            next: end,
            has_more: end < current.pages,
            status: current,
        })
    }
}
