//! Durable media attached to authenticated captured rows. No writes to WeChat.
use super::*;
use crate::conversation_media::{ConversationMediaKind, Selected};
use sha2::{Digest, Sha256};
use std::io::Write;
use zeroize::Zeroizing;
const CHUNK: usize = 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
pub struct ArchivedMedia {
    pub capture_id: String,
    pub page: u64,
    pub row: usize,
    pub row_digest: String,
    pub blob_id: String,
    pub chunks: u64,
    pub kind: ConversationMediaKind,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct MediaChunk {
    pub(crate) blob_id: String,
    pub(crate) index: u64,
    pub(crate) hex: String,
}
#[derive(Serialize)]
pub struct MediaRow {
    pub row: usize,
    pub kind: Option<ConversationMediaKind>,
    pub saved: Option<ArchivedMedia>,
    pub matches_message: bool,
    pub can_fetch: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct MediaProgress {
    pub page: u64,
    pub row: usize,
    pub saved: u64,
    pub skipped: u64,
    pub failed: u64,
    pub complete: bool,
    pub recent_errors: Vec<String>,
}
fn media_id(id: &str, page: u64, row: usize) -> String {
    format!("{id}:media:{page}:{row}")
}
fn row_digest(row: &CaptureRow) -> AppResult<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(row).map_err(|_| AppError::internal())?,
    )))
}
impl Application {
    fn capture_media_row(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
    ) -> AppResult<(Capture, CaptureRow)> {
        let (_, c): (_, Capture) = self.open_document(account, id, "source_capture")?;
        if page >= c.pages {
            return Err(AppError::new("NOT_FOUND", "归档页不存在"));
        }
        let (_, batch): (_, CaptureBatch) =
            self.open_document(account, &format!("{id}:{page}"), "capture_page")?;
        if batch.capture_id != id || batch.index != page {
            return Err(AppError::new("DOCUMENT_CORRUPTED", "归档页归属错误"));
        }
        let r = batch
            .rows
            .get(row)
            .ok_or_else(|| AppError::new("NOT_FOUND", "归档消息不存在"))?
            .clone();
        Ok((c, r))
    }
    fn capture_selected_media(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
    ) -> AppResult<Selected> {
        let (c, r) = self.capture_media_row(account, id, page, row)?;
        if c.imported || r.message.content_status == "edited_local" {
            return Err(AppError::new(
                "ARCHIVE_READ_ONLY",
                "导入或编辑后的消息不能用于原生媒体读取；已有媒体仍可导出",
            ));
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        if context.sender_id != c.login {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "请连接归档所属的微信账号",
            ));
        }
        let source = r
            .source_index
            .checked_sub(1)
            .and_then(|i| c.sources.get(i))
            .ok_or_else(|| AppError::new("DOCUMENT_CORRUPTED", "消息来源无效"))?;
        let (_, salt, fingerprint) = self.live_read_binding(account, &source.database)?;
        if salt != source.salt || fingerprint != source.fingerprint {
            return Err(AppError::new(
                "LIVE_SOURCE_REPLACED",
                "原消息来源已被替换，无法补齐媒体",
            ));
        }
        let binding = self.send_digest(
            account,
            "capture-media-binding",
            &(
                &context,
                revision,
                self.repository.key_revision(account)?,
                row_digest(&r)?,
            ),
        )?;
        Ok(Selected {
            database: Zeroizing::new(source.database.clone()),
            source: fingerprint,
            binding,
            username: Zeroizing::new(r.username),
            row: r.message,
        })
    }
    pub fn capture_media_page_local(
        &self,
        account: &str,
        id: &str,
        page: u64,
    ) -> AppResult<Vec<MediaRow>> {
        let (_, c): (_, Capture) = self.open_document(account, id, "source_capture")?;
        if page >= c.pages {
            return Err(AppError::new("NOT_FOUND", "归档页不存在"));
        }
        let (_, batch): (_, CaptureBatch) =
            self.open_document(account, &format!("{id}:{page}"), "capture_page")?;
        batch
            .rows
            .iter()
            .enumerate()
            .map(|(index, r)| {
                let saved = self.archived_media_optional(account, &media_id(id, page, index))?;
                let kind = crate::conversation_media::hint(&r.message, None).map(|h| h.kind);
                let matches_message = saved
                    .as_ref()
                    .map(|m| row_digest(r).map(|d| d == m.row_digest))
                    .transpose()?
                    .unwrap_or(true);
                Ok(MediaRow {
                    row: index,
                    kind,
                    saved,
                    matches_message,
                    can_fetch: !c.imported
                        && r.message.content_status != "edited_local"
                        && kind.is_some(),
                })
            })
            .collect()
    }
    fn archived_media_optional(&self, account: &str, id: &str) -> AppResult<Option<ArchivedMedia>> {
        match self.open_document(account, id, "capture_media") {
            Ok((_, m)) => Ok(Some(m)),
            Err(e) if e.code == "NOT_FOUND" => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn archive_capture_media_local(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
        request_id: &str,
    ) -> AppResult<ArchivedMedia> {
        let asset_id = media_id(id, page, row);
        if let Some(saved) = self.archived_media_optional(account, &asset_id)? {
            return Ok(saved);
        }
        let (_, original) = self.capture_media_row(account, id, page, row)?;
        let digest = row_digest(&original)?;
        let selected = self.capture_selected_media(account, id, page, row)?;
        let selected_row = CaptureRow {
            app_subtype:original.app_subtype,
            username: selected.username.to_string(),
            message: selected.row.clone(),
            source_index: original.source_index,
        };
        if row_digest(&selected_row)? != digest {
            return Err(AppError::new("CURSOR_STALE", "选择媒体期间归档消息已改变"));
        }
        let binding = selected.binding.clone();
        let saved = self.save_selected_media_local(
            account,
            selected,
            None,
            None,
            request_id,
            true,
            || self.capture_selected_media(account, id, page, row),
        )?;
        // The private temporary asset is consumed once. Release it on both commit
        // and failure so a long archive does not exhaust the preview-cache quota.
        let result = (|| {
            let contents = self.conversation_media_content_local(account, &saved.reference)?;
            if self.capture_selected_media(account, id, page, row)?.binding != binding {
                return Err(AppError::new("CURSOR_STALE", "归档来源或消息已改变"));
            }
            let bytes = &contents.file.bytes;
            let blob_id = uuid::Uuid::new_v4().to_string();
            let media = ArchivedMedia {
                capture_id: id.into(),
                page,
                row,
                row_digest: digest,
                blob_id: blob_id.clone(),
                chunks: bytes.len().div_ceil(CHUNK) as u64,
                kind: saved.reference.kind,
                name: contents.file.file_name.clone(),
                mime: contents.mime.into(),
                size: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
            };
            let lease = self.browse_lease(request_id)?.for_local_archive();
            let mut documents = bytes
                .chunks(CHUNK)
                .enumerate()
                .map(|(index, part)| {
                    lease.check()?;
                    self.seal_document_unchecked(
                        account,
                        &format!("{blob_id}:{index}"),
                        "media_chunk",
                        1,
                        &MediaChunk {
                            blob_id: blob_id.clone(),
                            index: index as u64,
                            hex: hex::encode(part),
                        },
                    )
                })
                .chain(std::iter::once_with(|| {
                    lease.check()?;
                    self.seal_document_unchecked(account, &asset_id, "capture_media", 1, &media)
                }));
            // One transaction: interruption never leaves a manifest or unreferenced chunks.
            self.repository.library_import(account, &mut documents)?;
            Ok(media)
        })();
        // These IDs are never shared with chat previews or pending sends. A cleanup
        // failure must not turn a committed archive into a failed/repeated download.
        let cleanup = match saved.reference.kind {
            ConversationMediaKind::Voice => {
                self.repository.delete_asset(account, &saved.reference.id)
            }
            ConversationMediaKind::Video => self
                .repository
                .delete_received_video(account, &saved.reference.id),
            _ => self.repository.delete_file(account, &saved.reference.id),
        };
        if let Err(error) = cleanup {
            eprintln!("archive temporary media cleanup failed: {}", error.code);
        }
        result
    }
    pub fn export_capture_media_local(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
        request_id: &str,
        out: &mut dyn Write,
    ) -> AppResult<ArchivedMedia> {
        let media = self
            .archived_media_optional(account, &media_id(id, page, row))?
            .ok_or_else(|| AppError::new("NOT_FOUND", "此消息媒体尚未归档"))?;
        if media.size > werc_domain::video::MAX_VIDEO_BYTES as u64
            || media.capture_id != id
            || media.page != page
            || media.row != row
            || media.chunks != media.size.div_ceil(CHUNK as u64)
        {
            return Err(AppError::new("DOCUMENT_CORRUPTED", "媒体清单无效"));
        }
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let mut hash = Sha256::new();
        let mut size = 0u64;
        for index in 0..media.chunks {
            lease.check()?;
            let (_, chunk): (_, MediaChunk) = self.open_document(
                account,
                &format!("{}:{index}", media.blob_id),
                "media_chunk",
            )?;
            if chunk.blob_id != media.blob_id || chunk.index != index {
                return Err(AppError::new("DOCUMENT_CORRUPTED", "媒体分块归属错误"));
            }
            let bytes = Zeroizing::new(
                hex::decode(&chunk.hex)
                    .map_err(|_| AppError::new("DOCUMENT_CORRUPTED", "媒体分块损坏"))?,
            );
            if bytes.len() > CHUNK {
                return Err(AppError::new("DOCUMENT_CORRUPTED", "媒体分块长度错误"));
            }
            size += bytes.len() as u64;
            hash.update(&bytes);
            out.write_all(&bytes)
                .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "媒体写入失败"))?;
        }
        if size != media.size || hex::encode(hash.finalize()) != media.sha256 {
            return Err(AppError::new(
                "DOCUMENT_CORRUPTED",
                "媒体摘要不一致；未发布文件",
            ));
        }
        Ok(media)
    }
    pub fn capture_media_progress_local(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<MediaProgress> {
        self.source_capture_status_local(account, id)?;
        match self.open_document(account, &format!("{id}:media-job"), "capture_media_job") {
            Ok((_, p)) => Ok(p),
            Err(e) if e.code == "NOT_FOUND" => Ok(MediaProgress::default()),
            Err(e) => Err(e),
        }
    }
    pub fn step_capture_media_local(
        &self,
        account: &str,
        id: &str,
        restart: bool,
        request_id: &str,
    ) -> AppResult<MediaProgress> {
        let job_id = format!("{id}:media-job");
        let (revision, mut progress) =
            match self.open_document::<MediaProgress>(account, &job_id, "capture_media_job") {
                Ok((h, p)) => (h.revision, p),
                Err(e) if e.code == "NOT_FOUND" => (0, MediaProgress::default()),
                Err(e) => return Err(e),
            };
        if restart {
            progress = MediaProgress::default();
        }
        let status = self.source_capture_status_local(account, id)?;
        if progress.page < status.pages {
            let rows = self.capture_media_page_local(account, id, progress.page)?;
            if let Some(r) = rows.get(progress.row) {
                if r.saved.is_some() {
                    progress.saved += 1;
                } else if !r.can_fetch {
                    progress.skipped += 1;
                } else {
                    match self.archive_capture_media_local(
                        account,
                        id,
                        progress.page,
                        progress.row,
                        request_id,
                    ) {
                        Ok(_) => progress.saved += 1,
                        Err(e)
                            if matches!(
                                e.code.as_str(),
                                "CONNECTION_CHANGED"
                                    | "RECEIVED_MEDIA_BUSY"
                                    | "CANCELED"
                                    | "QUERY_CANCELED"
                                    | "BROWSE_CANCELED"
                            ) =>
                        {
                            return Err(e);
                        }
                        Err(e) => {
                            progress.failed += 1;
                            progress.recent_errors.push(format!(
                                "第{}页第{}条：{} ({})",
                                progress.page + 1,
                                progress.row + 1,
                                e.message,
                                e.code
                            ));
                            if progress.recent_errors.len() > 20 {
                                progress.recent_errors.remove(0);
                            }
                        }
                    }
                }
                progress.row += 1;
            }
            if progress.row >= rows.len() {
                progress.page += 1;
                progress.row = 0;
            }
        }
        progress.complete = progress.page >= status.pages;
        let d = self.seal_document(
            account,
            &job_id,
            "capture_media_job",
            revision + 1,
            &progress,
        )?;
        self.repository.library_save(&[d], None)?;
        Ok(progress)
    }
}

#[derive(Serialize)]
pub struct MediaAudit {
    pub checked: u64,
    pub missing: u64,
    pub changed_messages: u64,
    pub damaged: u64,
    pub errors: Vec<String>,
}
impl Application {
    pub fn audit_capture_media_local(
        &self,
        account: &str,
        id: &str,
        request_id: &str,
    ) -> AppResult<MediaAudit> {
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let status = self.source_capture_status_local(account, id)?;
        let mut result = MediaAudit {
            checked: 0,
            missing: 0,
            changed_messages: 0,
            damaged: 0,
            errors: vec![],
        };
        for page in 0..status.pages {
            lease.check()?;
            for row in self.capture_media_page_local(account, id, page)? {
                lease.check()?;
                if row.saved.is_none() {
                    if row.kind.is_some() {
                        result.missing += 1;
                    }
                    continue;
                }
                if !row.matches_message {
                    result.changed_messages += 1;
                }
                match self.export_capture_media_local(
                    account,
                    id,
                    page,
                    row.row,
                    &uuid::Uuid::new_v4().to_string(),
                    &mut std::io::sink(),
                ) {
                    Ok(_) => result.checked += 1,
                    Err(e) => {
                        result.damaged += 1;
                        if result.errors.len() < 100 {
                            result.errors.push(format!(
                                "第{}页第{}条：{} ({})",
                                page + 1,
                                row.row + 1,
                                e.message,
                                e.code
                            ));
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaPosition {
    pub page: u64,
    pub row: usize,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaQuery {
    pub kind: Option<ConversationMediaKind>,
    pub saved: Option<bool>,
    pub username: Option<String>,
    pub from: Option<i64>,
    pub until: Option<i64>,
}
#[derive(Serialize)]
pub struct MediaCatalogItem {
    pub position: MediaPosition,
    pub username: String,
    pub timestamp: i64,
    pub media: MediaRow,
}
#[derive(Serialize)]
pub struct MediaCatalog {
    pub data: Vec<MediaCatalogItem>,
    pub next: Option<MediaPosition>,
}
impl Application {
    /// Bounded scan across saved pages, usable with imported/offline captures.
    pub fn capture_media_catalog_local(
        &self,
        account: &str,
        id: &str,
        query: &MediaQuery,
        after: Option<MediaPosition>,
        request_id: &str,
    ) -> AppResult<MediaCatalog> {
        if matches!((query.from,query.until),(Some(a),Some(b)) if a>=b)
            || query
                .username
                .as_deref()
                .is_some_and(|s| !werc_domain::live::valid_live_username(s))
        {
            return Err(AppError::new("INVALID_ARGUMENT", "会话标识无效"));
        }
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let status = self.source_capture_status_local(account, id)?;
        let mut pos = after.unwrap_or(MediaPosition { page: 0, row: 0 });
        let mut data = Vec::new();
        let mut scanned = 0;
        while pos.page < status.pages && scanned < 1000 && data.len() < 40 {
            lease.check()?;
            let (first, batch): (_, CaptureBatch) =
                self.open_document(account, &format!("{id}:{}", pos.page), "capture_page")?;
            let rows = self.capture_media_page_local(account, id, pos.page)?;
            // Both calls authenticate the same document; reject a concurrent edit.
            let (again, _): (_, CaptureBatch) =
                self.open_document(account, &format!("{id}:{}", pos.page), "capture_page")?;
            if first.revision != again.revision {
                return Err(AppError::new("DOCUMENT_CONFLICT", "浏览期间归档页已变化"));
            }
            if pos.row > batch.rows.len() {
                return Err(AppError::new("INVALID_ARGUMENT", "媒体续读位置无效"));
            }
            for media in rows.into_iter().skip(pos.row) {
                lease.check()?;
                let r = &batch.rows[media.row];
                let position = MediaPosition {
                    page: pos.page,
                    row: media.row,
                };
                pos.row = media.row + 1;
                scanned += 1;
                let actual = media.saved.as_ref().map(|m| m.kind).or(media.kind);
                if r.message.content_status != "deleted_local"
                    && query.from.is_none_or(|t| r.message.timestamp >= t)
                    && query.until.is_none_or(|t| r.message.timestamp < t)
                    && actual.is_some()
                    && query.kind.is_none_or(|k| actual == Some(k))
                    && query.saved.is_none_or(|s| s == media.saved.is_some())
                    && query.username.as_ref().is_none_or(|u| u == &r.username)
                {
                    data.push(MediaCatalogItem {
                        position,
                        username: r.username.clone(),
                        timestamp: r.message.timestamp,
                        media,
                    });
                }
                if scanned >= 1000 || data.len() >= 40 {
                    break;
                }
            }
            if pos.row >= batch.rows.len() {
                pos.page += 1;
                pos.row = 0;
            }
        }
        Ok(MediaCatalog {
            data,
            next: (pos.page < status.pages).then_some(pos),
        })
    }
    pub fn capture_media_content_local(
        &self,
        account: &str,
        id: &str,
        page: u64,
        row: usize,
        request_id: &str,
    ) -> AppResult<crate::conversation_media::ConversationMediaContents> {
        let mut bytes = Zeroizing::new(Vec::new());
        let media =
            self.export_capture_media_local(account, id, page, row, request_id, &mut *bytes)?;
        // Imported manifest text never controls an executable content type.
        let mime = match media.kind {
            ConversationMediaKind::Image => "image/png",
            ConversationMediaKind::Voice => "audio/wav",
            ConversationMediaKind::Video => "video/mp4",
            ConversationMediaKind::Emoticon if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => {
                "image/png"
            }
            ConversationMediaKind::Emoticon => "image/gif",
            ConversationMediaKind::File => "application/octet-stream",
        };
        Ok(crate::conversation_media::ConversationMediaContents {
            file: werc_domain::file::FileContents {
                file_name: media.name,
                bytes,
            },
            mime,
        })
    }
}
