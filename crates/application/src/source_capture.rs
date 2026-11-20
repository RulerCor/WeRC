//! Resumable all-conversation source reading. Checkpoints and pages commit together.
//! Source rows retain their own identity; they are not native hook ACKs or delivery proofs.
use crate::Application;
use serde::{Deserialize, Serialize};
use werc_domain::{AppError, AppResult, library::*, live::*};
use zeroize::Zeroize;
#[path="capture_names.rs"]
mod names;
pub use names::CaptureNamesSummary;

#[derive(Serialize, Deserialize)]
struct Source {
    database: String,
    salt: [u8; 16],
    fingerprint: String,
}
#[derive(Serialize, Deserialize)]
struct Capture {
    #[serde(default)]
    names:names::CaptureNames,
    #[serde(default)]
    imported: bool,
    login: String,
    started_at: String,
    query: LiveMessageQuery,
    sources: Vec<Source>,
    source_index: usize,
    after: Option<LiveSearchPosition>,
    pages: u64,
    rows: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CaptureRow {
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub app_subtype:Option<u32>,
    pub username: String,
    pub message: LiveMessage,
    pub source_index: usize,
}
#[derive(Serialize, Deserialize)]
struct CaptureBatch {
    capture_id: String,
    index: u64,
    rows: Vec<CaptureRow>,
}
#[derive(Serialize)]
pub struct CaptureStatus {
    pub imported: bool,
    pub can_resume: bool,
    pub id: String,
    pub started_at: String,
    pub rows: u64,
    pub pages: u64,
    pub source_count: usize,
    pub source_index: usize,
    pub complete_selected_sources: bool,
    pub cross_page_snapshot: bool,
    pub native_gaps_closed: bool,
    pub query: LiveMessageQuery,
}
#[derive(Serialize)]
pub struct CaptureList {
    pub data: Vec<CaptureStatus>,
    pub next_cursor: Option<i64>,
}
#[derive(Serialize)]
pub struct CapturePage {
    pub revision: i64,
    pub rows: Vec<CaptureRow>,
    pub next_page: Option<u64>,
}
fn status(h: &DocumentHeader, c: &Capture) -> CaptureStatus {
    CaptureStatus {
        imported: c.imported,
        can_resume: !c.imported && c.source_index < c.sources.len(),
        id: h.id.clone(),
        started_at: c.started_at.clone(),
        rows: c.rows,
        pages: c.pages,
        source_count: c.sources.len(),
        source_index: c.source_index,
        complete_selected_sources: c.source_index == c.sources.len(),
        cross_page_snapshot: false,
        native_gaps_closed: false,
        query: c.query.clone(),
    }
}
impl Application {
    pub(crate) fn capture_login_local(&self, account: &str, id: &str) -> AppResult<String> {
        let (_, capture): (_, Capture) = self.open_document(account, id, "source_capture")?;
        Ok(capture.login.clone())
    }
    /// Restartable scheduled capture, tied to its own durable job identity.
    pub fn fork_source_capture_local(
        &self,
        account: &str,
        template: &str,
        id: &str,
        from: Option<i64>,
        until: i64,
    ) -> AppResult<CaptureStatus> {
        match self.source_capture_status_local(account, id) {
            Ok(value) => return Ok(value),
            Err(e) if e.code == "NOT_FOUND" => {}
            Err(e) => return Err(e),
        }
        let (_, mut capture): (_, Capture) =
            self.open_document(account, template, "source_capture")?;
        if capture.imported {
            return Err(AppError::new(
                "ARCHIVE_READ_ONLY",
                "导入副本没有本机来源，不能用于自动补读计划",
            ));
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        if context.sender_id != capture.login {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "计划账号与当前登录者不同",
            ));
        }
        let key_revision = self.repository.key_revision(account)?;
        capture.query.from_timestamp = from;
        capture.query.until_timestamp = Some(until);
        if !capture.query.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "计划读取范围无效"));
        }
        capture.started_at = chrono::Utc::now().to_rfc3339();
        capture.source_index = 0;
        capture.after = None;
        capture.pages = 0;
        capture.rows = 0;
        let doc = self.seal_document(account, id, "source_capture", 1, &capture)?;
        let result = status(&doc.header, &capture);
        self.repository.library_save(
            &[doc],
            Some(&DocumentConnection {
                revision,
                key_revision,
            }),
        )?;
        Ok(result)
    }
    pub fn source_capture_status_local(&self, account: &str, id: &str) -> AppResult<CaptureStatus> {
        let (h, c): (_, Capture) = self.open_document(account, id, "source_capture")?;
        Ok(status(&h, &c))
    }
    pub fn start_source_capture_local(
        &self,
        account: &str,
        view_id: &str,
        from: Option<i64>,
        until: Option<i64>,
    ) -> AppResult<CaptureStatus> {
        let _guard = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源操作正在执行"))?;
        let mut query = LiveMessageQuery::default();
        query.from_timestamp = from;
        query.until_timestamp = Some(until.unwrap_or_else(|| chrono::Utc::now().timestamp() + 1));
        if !query.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "补读日期范围无效"));
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let key_revision = self.repository.key_revision(account)?;
        let sources = self.with_search_parent(account, view_id, |_, _, sources, _| {
            Ok(sources
                .iter()
                .map(|s| Source {
                    database: s.database.to_string(),
                    salt: s.salt,
                    fingerprint: s.fingerprint.clone(),
                })
                .collect())
        })?;
        let capture = Capture {
            names:Default::default(),
            imported: false,
            login: context.sender_id.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
            query,
            sources,
            source_index: 0,
            after: None,
            pages: 0,
            rows: 0,
        };
        let doc = self.seal_document(
            account,
            &uuid::Uuid::new_v4().to_string(),
            "source_capture",
            1,
            &capture,
        )?;
        let result = status(&doc.header, &capture);
        self.repository.library_save(
            &[doc],
            Some(&DocumentConnection {
                revision,
                key_revision,
            }),
        )?;
        Ok(result)
    }
    pub fn step_source_capture_local(
        &self,
        account: &str,
        id: &str,
        request_id: &str,
    ) -> AppResult<CaptureStatus> {
        let _guard = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源操作正在执行"))?;
        let lease = self.browse_lease(request_id)?;
        let (h, mut capture): (_, Capture) = self.open_document(account, id, "source_capture")?;
        if capture.source_index == capture.sources.len() {
            return Ok(status(&h, &capture));
        }
        if capture.imported {
            return Err(AppError::new(
                "ARCHIVE_READ_ONLY",
                "导入归档可浏览、导出和统计；继续补读请从本机消息来源新建任务",
            ));
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let key_revision = self.repository.key_revision(account)?;
        if context.sender_id != capture.login {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "补读账号与当前登录者不同",
            ));
        }
        let source = capture
            .sources
            .get(capture.source_index)
            .ok_or_else(|| AppError::new("DOCUMENT_CORRUPTED", "补读进度无效"))?;
        let (binding, salt, fingerprint) = self.live_read_binding(account, &source.database)?;
        if salt != source.salt || fingerprint != source.fingerprint {
            return Err(AppError::new(
                "LIVE_SOURCE_REPLACED",
                "原补读来源已被替换，请重新开始；已有资料保留",
            ));
        }
        let keys = self.live_keys_local(account)?;
        let key = crate::keyring::key_for(&keys, &source.salt)?;
        let selection = LiveSearchSelection {
            query: capture.query.clone(),
            after: capture.after.take(),
        };
        lease.check()?;
        let window = self.provider.search_live_messages(
            account,
            &source.database,
            &key,
            &source.fingerprint,
            &selection,
            &lease.cancel,
        )?;
        if !window.valid(&selection) {
            return Err(AppError::new(
                "LIVE_PROTOCOL_INVALID",
                "补读结果或续页位置无效",
            ));
        }
        lease.check()?;
        if self.live_read_binding(account, &source.database)?.0 != binding {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "读取期间身份或来源变化，未推进补读",
            ));
        }
        let batch = CaptureBatch {
            capture_id: id.into(),
            index: capture.pages,
            rows: window
                .rows
                .into_iter()
                .map(|hit| CaptureRow {
                    app_subtype: capture_subtype(&hit.message),
                    username: hit.username.clone(),
                    message: hit.message.clone(),
                    source_index: capture.source_index + 1,
                })
                .collect(),
        };
        capture.rows += batch.rows.len() as u64;
        capture.after = window.next;
        if capture.after.is_none() {
            capture.source_index += 1;
        }
        capture.pages += 1;
        let page = self.seal_document(
            account,
            &format!("{id}:{}", batch.index),
            "capture_page",
            1,
            &batch,
        )?;
        let job = self.seal_document(account, id, "source_capture", h.revision + 1, &capture)?;
        let result = status(&job.header, &capture);
        self.repository.library_save(
            &[page, job],
            Some(&DocumentConnection {
                revision,
                key_revision,
            }),
        )?;
        Ok(result)
    }
    pub fn source_captures_local(
        &self,
        account: &str,
        before: Option<i64>,
    ) -> AppResult<CaptureList> {
        self.ensure_account(account)?;
        let page = self
            .repository
            .library_list(account, "source_capture", before, 20)?;
        let data = page
            .data
            .iter()
            .map(|h| {
                self.open_document::<Capture>(account, &h.id, "source_capture")
                    .map(|(h, c)| status(&h, &c))
            })
            .collect::<AppResult<_>>()?;
        Ok(CaptureList {
            data,
            next_cursor: page.next_cursor,
        })
    }
    pub fn source_capture_page_local(
        &self,
        account: &str,
        id: &str,
        index: u64,
    ) -> AppResult<CapturePage> {
        let (_, capture): (_, Capture) = self.open_document(account, id, "source_capture")?;
        if index >= capture.pages {
            return Err(AppError::new("NOT_FOUND", "补读页尚未保存"));
        }
        let (page_header, mut batch): (_, CaptureBatch) =
            self.open_document(account, &format!("{id}:{index}"), "capture_page")?;
        if batch.capture_id != id || batch.index != index {
            return Err(AppError::new("DOCUMENT_CORRUPTED", "补读页归属无效"));
        }
        for row in &mut batch.rows {
            row.app_subtype=capture_subtype(&row.message);
            row.message.metadata.zeroize();
        }
        Ok(CapturePage {
            revision: page_header.revision,
            rows: batch.rows,
            next_page: (index + 1 < capture.pages).then_some(index + 1),
        })
    }
}

fn capture_subtype(message:&LiveMessage)->Option<u32>{
    werc_domain::message_card::app_subtype(message.message_type.parse().ok()?,message.metadata.as_deref().unwrap_or("")).ok().flatten()
}

#[path = "capture_edits.rs"]
mod edits;
pub use edits::*;

#[path = "capture_media.rs"]
pub(crate) mod media;
pub use media::*;

#[path = "capture_bundle.rs"]
mod bundle;
pub use bundle::*;

#[path = "capture_transcripts.rs"]
mod transcripts;

#[path = "capture_records.rs"]
mod records;
pub use records::*;
