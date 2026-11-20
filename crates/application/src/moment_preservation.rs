//! Retain observed local moments in an encrypted independent copy. This does not
//! prevent WeChat/server deletion or recover content never observed on this PC.
use crate::{Application, backfill::BoundSource};
use serde::{Deserialize, Serialize};
use werc_domain::{AppError, AppResult, library::*, moments::*};
const STATE: &str = "moment-preservation-v1";
const KIND: &str = "moment_preserved";

#[derive(Default, Serialize, Deserialize)]
struct Preservation {
    enabled: bool,
    login: String,
    source: Option<BoundSource>,
    after: Option<LiveMomentContinuation>,
    saved: u64,
    last_completed: Option<i64>,
    next_run: i64,
    error: Option<String>,
    #[serde(default)]
    media_pending: std::collections::VecDeque<MomentMediaSelection>,
    #[serde(default)]
    media_failed: u64,
    #[serde(default)]
    media_errors: Vec<String>,
    #[serde(default)]
    page_finished: bool,
}
#[derive(Serialize)]
pub struct PreservationStatus {
    pub enabled: bool,
    pub saved: u64,
    pub scanning: bool,
    pub last_completed: Option<i64>,
    pub error: Option<String>,
    pub media_saved: u64,
    pub media_pending: usize,
    pub media_failed: u64,
    pub media_errors: Vec<String>,
}
impl Preservation {
    fn status(&self) -> PreservationStatus {
        PreservationStatus {
            enabled: self.enabled,
            saved: self.saved,
            scanning: self.enabled && self.source.is_some(),
            last_completed: self.last_completed,
            error: self.error.clone(),
            media_saved: 0,
            media_pending: self.media_pending.len(),
            media_failed: self.media_failed,
            media_errors: self.media_errors.clone(),
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct PreservedMoment {
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub login: Option<String>,
    pub first_observed: i64,
    pub last_changed: i64,
    pub moment: LiveMoment,
    digest: String,
}
#[derive(Serialize)]
pub struct PreservedMomentPage {
    pub data: Vec<PreservedMoment>,
    pub next_cursor: Option<i64>,
    pub scanned: usize,
}
impl Application {
    /// Manual media capture also retains the authenticated display row. Returned
    /// documents join the media transaction; no placeholder can escape on failure.
    pub(crate) fn new_media_moment_document(
        &self,
        account: &str,
        row: &LiveMoment,
    ) -> AppResult<Option<ProtectedDocument>> {
        let (context, _) = self.checked_send_context(account, "filehelper")?;
        let id = format!(
            "moment-{}",
            self.send_digest(
                account,
                "preserved-moment-id",
                &(&context.sender_id, &row.anchor.id, &row.anchor.username)
            )?
        );
        match self.open_document::<PreservedMoment>(account, &id, KIND) {
            Ok(_) => return Ok(None),
            Err(e) if e.code == "NOT_FOUND" => {}
            Err(e) => return Err(e),
        }
        let now = chrono::Utc::now().timestamp();
        let item = PreservedMoment {
            login: Some(context.sender_id.clone()),
            first_observed: now,
            last_changed: now,
            moment: row.clone(),
            digest: self.send_digest(
                account,
                "preserved-moment-content",
                &(
                    &row.anchor.id,
                    &row.anchor.username,
                    &row.content,
                    &row.content_status,
                ),
            )?,
        };
        Ok(Some(
            self.seal_document_unchecked(account, &id, KIND, 1, &item)?,
        ))
    }
    fn preservation(&self, account: &str) -> AppResult<(i64, Preservation)> {
        match self.open_document::<Preservation>(account, STATE, "moment_preservation") {
            Ok((h, s)) => Ok((h.revision, s)),
            Err(e) if e.code == "NOT_FOUND" => Ok((0, Preservation::default())),
            Err(e) => Err(e),
        }
    }
    pub fn moment_preservation_status_local(&self, account: &str) -> AppResult<PreservationStatus> {
        let mut status = self.preservation(account)?.1.status();
        status.saved = self.repository.library_count(account, KIND)?;
        status.media_saved = self.repository.library_count(account, "moment_media")?;
        Ok(status)
    }
    pub fn configure_moment_preservation_local(
        &self,
        account: &str,
        enabled: bool,
    ) -> AppResult<PreservationStatus> {
        if !enabled {
            let jobs = self.browse_jobs.lock().map_err(|_| AppError::internal())?;
            if let Some(job) = jobs.get("moment-preservation-media") {
                job.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _guard = self
            .moment_preservation_guard
            .lock()
            .map_err(|_| AppError::internal())?;
        let (revision, mut s) = self.preservation(account)?;
        if enabled {
            let (context, _) = self.checked_send_context(account, "filehelper")?;
            if !s.login.is_empty() && s.login != context.sender_id {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "当前登录者与保留资料所属账号不同",
                ));
            }
            s.login = context.sender_id;
        }
        s.enabled = enabled;
        s.next_run = 0;
        s.error = None;
        let d = self.seal_document(account, STATE, "moment_preservation", revision + 1, &s)?;
        self.repository.library_save(&[d], None)?;
        self.moment_preservation_status_local(account)
    }
    /// One local-cache page per background step. Row and counter updates are
    /// transactional; repeating a page after restart does not duplicate records.
    pub fn step_moment_preservation_local(&self, account: &str) -> AppResult<PreservationStatus> {
        let _guard = self
            .moment_preservation_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "正在保存朋友圈"))?;
        let (mut revision, mut s) = self.preservation(account)?;
        let now = chrono::Utc::now().timestamp();
        if !s.enabled || s.next_run > now {
            return self.moment_preservation_status_local(account);
        }
        let _history = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源读取正在执行"))?;
        let (context, connection) = self.checked_send_context(account, "filehelper")?;
        if context.sender_id != s.login {
            return Err(AppError::new("CONNECTION_CHANGED", "朋友圈保留账号已变化"));
        }
        let key_revision = self.repository.key_revision(account)?;
        let connection = DocumentConnection {
            revision: connection,
            key_revision,
        };
        let result = (|| {
            if s.source.is_none() {
                s.media_failed = 0;
                s.media_errors.clear();
                s.page_finished = false;
                s.source = self
                    .enrolled_message_sources(account, &s.login)?
                    .into_iter()
                    .next();
                s.after = None;
            }
            let source = s.source.as_ref().ok_or_else(|| {
                AppError::new(
                    "SOURCE_REQUIRED",
                    "连接微信并准备消息来源后会自动保留朋友圈",
                )
            })?;
            let (binding, salt, fingerprint) = self.live_read_binding(account, &source.database)?;
            if salt != source.salt || fingerprint != source.fingerprint {
                return Err(AppError::new(
                    "LIVE_SOURCE_REPLACED",
                    "朋友圈读取来源已变化",
                ));
            }
            if let Some(selection) = s.media_pending.front() {
                match self.retain_moment_media_from_source(account, source, selection) {
                    Ok(_) => {}
                    Err(e)
                        if matches!(
                            e.code.as_str(),
                            "QUERY_CANCELED"
                                | "QUERY_TIMEOUT"
                                | "BUSY"
                                | "CONNECTION_CHANGED"
                                | "LIVE_SOURCE_REPLACED"
                                | "NATIVE_CONTEXT_CHANGED"
                        ) =>
                    {
                        return Err(e);
                    }
                    Err(e) => {
                        s.media_failed += 1;
                        let message = format!(
                            "{} / {} / {:?}: {}",
                            selection.anchor.id,
                            selection.slot + 1,
                            selection.variant,
                            e.code
                        );
                        if s.media_errors.len() == 8 {
                            s.media_errors.remove(0);
                        }
                        s.media_errors.push(message);
                    }
                }
                s.media_pending.pop_front();
                s.error = None;
                if s.media_pending.is_empty() && s.page_finished {
                    s.source = None;
                    s.last_completed = Some(now);
                    s.next_run = now + 300;
                }
                return Ok(());
            }
            let lease = self.browse_lease("moment-preservation")?;
            let (sns_fingerprint, window) = self.provider.read_live_moments(
                account,
                &source.database,
                &fingerprint,
                s.after.as_ref(),
                &lease.cancel,
            )?;
            if !window.valid(s.after.as_ref().map(|a| &a.anchor))
                || sns_fingerprint.len() != 64
                || !sns_fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(AppError::new(
                    "LIVE_PROTOCOL_INVALID",
                    "朋友圈保留读取结果无效",
                ));
            }
            lease.check()?;
            if self.live_read_binding(account, &source.database)?.0 != binding {
                return Err(AppError::new("CONNECTION_CHANGED", "读取期间来源变化"));
            }
            for row in &window.rows {
                lease.check()?;
                if let Some(content) = &row.content {
                    for media in &content.details.media {
                        let mut variants = vec![MomentMediaVariant::Original];
                        if media.live_photo {
                            variants.push(MomentMediaVariant::LiveVideo);
                        }
                        for variant in variants {
                            let selection = MomentMediaSelection {
                                anchor: row.anchor.clone(),
                                slot: media.slot,
                                variant,
                            };
                            if self
                                .cached_moment_media_local(account, &selection)?
                                .is_none()
                            {
                                s.media_pending.push_back(selection);
                            }
                        }
                    }
                }
                let saved_before = s.saved;
                let id = format!(
                    "moment-{}",
                    self.send_digest(
                        account,
                        "preserved-moment-id",
                        &(&s.login, &row.anchor.id, &row.anchor.username)
                    )?
                );
                // Rowid is a local database location; copying/rebuilding the source
                // must not create duplicate preserved moments.
                let digest = self.send_digest(
                    account,
                    "preserved-moment-content",
                    &(
                        &row.anchor.id,
                        &row.anchor.username,
                        &row.content,
                        &row.content_status,
                    ),
                )?;
                let (old_revision, first) =
                    match self.open_document::<PreservedMoment>(account, &id, KIND) {
                        Ok((h, old)) => {
                            // Never replace an already readable copy with a later unreadable placeholder.
                            if old.digest == digest
                                || (old.moment.content.is_some() && row.content.is_none())
                            {
                                continue;
                            }
                            (h.revision, old.first_observed)
                        }
                        Err(e) if e.code == "NOT_FOUND" => {
                            s.saved += 1;
                            (0, now)
                        }
                        Err(e) => return Err(e),
                    };
                let item = PreservedMoment {
                    login: Some(s.login.clone()),
                    first_observed: first,
                    last_changed: now,
                    moment: row.clone(),
                    digest,
                };
                let document = self.seal_document(account, &id, KIND, old_revision + 1, &item)?;
                let state =
                    self.seal_document(account, STATE, "moment_preservation", revision + 1, &s)?;
                if let Err(e) = self
                    .repository
                    .library_save(&[document, state], Some(&connection))
                {
                    s.saved = saved_before;
                    return Err(e);
                }
                revision += 1;
            }
            s.after = if window.has_more {
                Some(LiveMomentContinuation {
                    source_fingerprint: sns_fingerprint,
                    anchor: window
                        .rows
                        .last()
                        .ok_or_else(AppError::internal)?
                        .anchor
                        .clone(),
                })
            } else {
                None
            };
            s.page_finished = !window.has_more;
            if !window.has_more && s.media_pending.is_empty() {
                s.source = None;
                s.last_completed = Some(now);
                s.next_run = now + 300;
            }
            s.error = None;
            Ok::<_, AppError>(())
        })();
        if let Err(error) = result {
            // Persist useful error state and retry from the cache head. A disappearing
            // anchor may mean deletion; all previously preserved copies remain intact.
            let retry_current = matches!(
                error.code.as_str(),
                "QUERY_CANCELED" | "QUERY_TIMEOUT" | "BUSY"
            );
            s.error = Some(error.code);
            s.next_run = now + 30;
            if !retry_current {
                s.after = None;
                s.source = None;
                s.media_pending.clear();
                s.page_finished = false;
            }
        }
        let state = self.seal_document(account, STATE, "moment_preservation", revision + 1, &s)?;
        self.repository.library_save(&[state], Some(&connection))?;
        self.moment_preservation_status_local(account)
    }
    pub fn preserved_moments_local(
        &self,
        account: &str,
        before: Option<i64>,
        query: &MomentQuery,
    ) -> AppResult<PreservedMomentPage> {
        self.ensure_account(account)?;
        if !query.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "朋友圈筛选无效"));
        }
        let page = self.repository.library_list(account, KIND, before, 30)?;
        let mut data = Vec::new();
        let scanned = page.data.len();
        for header in page.data {
            let (_, item) = self.open_document::<PreservedMoment>(account, &header.id, KIND)?;
            if query.matches(&item.moment) {
                data.push(item);
            }
        }
        Ok(PreservedMomentPage {
            data,
            next_cursor: page.next_cursor,
            scanned,
        })
    }
    pub fn export_preserved_moments_local(
        &self,
        account: &str,
        query: &MomentQuery,
        format: crate::live_moments::MomentExportFormat,
        request: &str,
    ) -> AppResult<crate::live_moments::MomentExport> {
        use crate::live_moments::{MomentExport, MomentExportFormat as F, MomentExportSummary};
        use std::io::Write;
        let lease = self.browse_lease(request)?;
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        let mut before = None;
        let (mut count, mut scanned, mut unreadable, mut missing) = (0, 0, 0, 0u64);
        match format {
            F::Json=>bytes.extend_from_slice(b"{\"mode\":\"preserved_local_moments\",\"complete_timeline\":false,\"rows\":["),
            F::Html=>bytes.extend_from_slice("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none';style-src 'unsafe-inline';base-uri 'none'\"><title>WeRC 已保留朋友圈</title><style>body{font:16px system-ui;max-width:850px;margin:auto;padding:24px}pre{white-space:pre-wrap;overflow-wrap:anywhere}article{border-bottom:1px solid #ccc}</style><h1>已保留的朋友圈</h1><p>本机已观察到的独立副本，可能与当前微信不同。此导出不含媒体文件。</p>".as_bytes()),
            _=>bytes.extend_from_slice("已保留的朋友圈（本机副本；不含媒体文件）\n\n".as_bytes()),
        }
        loop {
            lease.check()?;
            let page = self.preserved_moments_local(account, before, query)?;
            scanned += page.scanned;
            for item in page.data {
                let row = &item.moment;
                if row.content.is_none() {
                    unreadable += 1;
                }
                missing += u64::from(row.content.as_ref().map(|c| c.media_count).unwrap_or(0));
                let text = row
                    .content
                    .as_ref()
                    .map(|c| {
                        format!(
                            "{}\n{}",
                            c.text,
                            super::live_moments::details_text(&c.details)
                        )
                    })
                    .unwrap_or_else(|| "内容不可读".into());
                let display = format!(
                    "作者：{}\n动态：{}\n发布时间：{}\n首次保留：{}\n最后内容变化：{}\n{}\n",
                    row.anchor.username,
                    row.anchor.id,
                    row.content
                        .as_ref()
                        .map(|c| c.timestamp.to_string())
                        .unwrap_or_default(),
                    item.first_observed,
                    item.last_changed,
                    text
                );
                match format {
                    F::Json => {
                        if count > 0 {
                            bytes.push(b',');
                        }
                        serde_json::to_writer(&mut *bytes, &item)
                            .map_err(|_| AppError::internal())?;
                    }
                    F::Html => {
                        let escaped = display
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;");
                        write!(&mut *bytes, "<article><pre>{escaped}</pre></article>")
                            .map_err(|_| AppError::internal())?;
                    }
                    F::Txt => bytes.extend_from_slice(display.as_bytes()),
                    F::Md => {
                        for line in display.lines() {
                            writeln!(&mut *bytes, "    {line}")
                                .map_err(|_| AppError::internal())?;
                        }
                        bytes.extend_from_slice(b"\n---\n\n");
                    }
                }
                count += 1;
                if bytes.len() > 32 * 1024 * 1024 {
                    return Err(AppError::new("EXPORT_LIMIT", "导出超过32MiB，请缩小范围"));
                }
            }
            before = page.next_cursor;
            if before.is_none() {
                break;
            }
        }
        match format {
            F::Json => bytes.extend_from_slice(b"]}"),
            F::Html => bytes.extend_from_slice(b"</html>"),
            _ => {}
        }
        lease.check()?;
        Ok(MomentExport {
            summary: MomentExportSummary {
                rows: count,
                scanned,
                bytes: bytes.len(),
                missing_media: missing,
                unreadable,
                complete_timeline: false,
                cross_page_snapshot: false,
            },
            bytes,
        })
    }
}
