//! Account-bound SNS media, retained independently of the live cache.
use crate::Application;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, moments::*};
use zeroize::Zeroizing;
const CHUNK: usize = 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
pub struct MomentMediaManifest {
    pub id: String,
    pub moment_id: String,
    pub username: String,
    pub slot: usize,
    pub variant: MomentMediaVariant,
    pub blob_id: String,
    pub chunks: usize,
    pub size: usize,
    pub sha256: String,
    pub mime: String,
    pub file_name: String,
    pub saved_at: i64,
    pub source_md5_matches: Option<bool>,
}
fn key(s: &MomentMediaSelection) -> String {
    let variant = match s.variant {
        MomentMediaVariant::Original => "original",
        MomentMediaVariant::Thumbnail => "thumbnail",
        MomentMediaVariant::LiveVideo => "live_video",
    };
    format!(
        "snsmedia-{}",
        hex::encode(Sha256::digest(format!(
            "{}\0{}\0{}\0{variant}",
            s.anchor.id, s.anchor.username, s.slot
        )))
    )
}
fn matches(m: &MomentMediaManifest, s: &MomentMediaSelection) -> bool {
    m.moment_id == s.anchor.id
        && m.username == s.anchor.username
        && m.slot == s.slot
        && m.variant == s.variant
}
fn invalid() -> AppError {
    AppError::new("DOCUMENT_CORRUPTED", "朋友圈媒体副本校验失败")
}
impl Application {
    /// Repair older manual captures which have media but no display row. Only
    /// authenticated rows actually read from the current source can be restored.
    pub(crate) fn backfill_cached_moment_rows(
        &self,
        account: &str,
        rows: &[LiveMoment],
    ) -> AppResult<()> {
        if rows.is_empty() || self.repository.library_count(account, "moment_media")? == 0 {
            return Ok(());
        }
        let mut wanted = std::collections::BTreeSet::new();
        let mut before = None;
        loop {
            let page = self
                .repository
                .library_list(account, "moment_media", before, 100)?;
            for h in page.data {
                let (_, m) =
                    self.open_document::<MomentMediaManifest>(account, &h.id, "moment_media")?;
                if rows
                    .iter()
                    .any(|r| r.anchor.id == m.moment_id && r.anchor.username == m.username)
                {
                    wanted.insert((m.moment_id, m.username));
                }
            }
            before = page.next_cursor;
            if before.is_none() {
                break;
            }
        }
        let mut documents = Vec::new();
        for row in rows {
            if wanted.contains(&(row.anchor.id.clone(), row.anchor.username.clone())) {
                if let Some(d) = self.new_media_moment_document(account, row)? {
                    documents.push(d);
                }
            }
        }
        if !documents.is_empty() {
            self.repository
                .library_import(account, &mut documents.into_iter().map(Ok))?;
        }
        Ok(())
    }
    pub fn cached_moment_media_local(
        &self,
        account: &str,
        selection: &MomentMediaSelection,
    ) -> AppResult<Option<MomentMediaManifest>> {
        self.ensure_account(account)?;
        if !selection.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "朋友圈媒体选择无效"));
        }
        match self.open_document::<MomentMediaManifest>(account, &key(selection), "moment_media") {
            Ok((_, m)) => {
                if !matches(&m, selection) {
                    return Err(invalid());
                }
                return Ok(Some(m));
            }
            Err(e) if e.code == "NOT_FOUND" => {}
            Err(e) => return Err(e),
        }
        // Portable backups assign new document IDs; locate the preserved identity.
        let mut before = None;
        loop {
            let page = self
                .repository
                .library_list(account, "moment_media", before, 100)?;
            for h in &page.data {
                let (_, m) =
                    self.open_document::<MomentMediaManifest>(account, &h.id, "moment_media")?;
                if matches(&m, selection) {
                    return Ok(Some(m));
                }
            }
            let Some(next) = page.next_cursor else {
                return Ok(None);
            };
            before = Some(next);
        }
    }
    pub fn fetch_moment_media_local(
        &self,
        account: &str,
        view: &str,
        selection: &MomentMediaSelection,
        request: &str,
    ) -> AppResult<MomentMediaManifest> {
        let _guard = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源读取正在执行"))?;
        if let Some(saved) = self.cached_moment_media_local(account, selection)? {
            return Ok(saved);
        }
        let lease = self.browse_lease(request)?;
        let (binding, source) =
            self.with_search_parent(account, view, |binding, _, sources, _| {
                Ok((
                    binding.to_owned(),
                    sources.first().ok_or_else(invalid)?.clone(),
                ))
            })?;
        let content = self.provider.read_moment_media(
            account,
            &source.database,
            &source.fingerprint,
            selection,
            &lease.cancel,
        )?;
        lease.check()?;
        self.with_search_parent(account, view, |current, _, sources, _| {
            if current != binding
                || sources
                    .first()
                    .is_none_or(|s| s.fingerprint != source.fingerprint)
            {
                Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "媒体读取期间来源已变化",
                ))
            } else {
                Ok(())
            }
        })?;
        self.store_moment_media_local(account, selection, content, &lease)
    }
    pub(crate) fn retain_moment_media_from_source(
        &self,
        account: &str,
        source: &crate::backfill::BoundSource,
        selection: &MomentMediaSelection,
    ) -> AppResult<MomentMediaManifest> {
        if let Some(saved) = self.cached_moment_media_local(account, selection)? {
            return Ok(saved);
        }
        let lease = self.browse_lease("moment-preservation-media")?;
        let (binding, salt, fingerprint) = self.live_read_binding(account, &source.database)?;
        if salt != source.salt || fingerprint != source.fingerprint {
            return Err(AppError::new("LIVE_SOURCE_REPLACED", "朋友圈媒体来源变化"));
        }
        let content = self.provider.read_moment_media(
            account,
            &source.database,
            &source.fingerprint,
            selection,
            &lease.cancel,
        )?;
        lease.check()?;
        if self.live_read_binding(account, &source.database)?.0 != binding {
            return Err(AppError::new("CONNECTION_CHANGED", "媒体读取期间身份变化"));
        }
        self.store_moment_media_local(account, selection, content, &lease)
    }
    fn store_moment_media_local(
        &self,
        account: &str,
        selection: &MomentMediaSelection,
        content: MomentMediaBytes,
        lease: &crate::history::QueryLease<'_>,
    ) -> AppResult<MomentMediaManifest> {
        if !content.moment.valid()
            || content.moment.anchor.id != selection.anchor.id
            || content.moment.anchor.username != selection.anchor.username
            || content.moment.anchor.row_id != selection.anchor.row_id
            || content.moment.content.as_ref().is_none_or(|c| {
                !c.details.media.iter().any(|m| {
                    m.slot == selection.slot
                        && (selection.variant != MomentMediaVariant::LiveVideo || m.live_photo)
                })
            })
            || content.bytes.is_empty()
            || content.bytes.len() > werc_domain::video::MAX_VIDEO_BYTES
            || !matches!(
                (content.mime, content.extension),
                ("image/png", "png") | ("video/mp4", "mp4")
            )
        {
            return Err(AppError::new("MOMENT_MEDIA_INVALID", "媒体读取结果无效"));
        }
        let manifest = MomentMediaManifest {
            id: key(selection),
            moment_id: selection.anchor.id.clone(),
            username: selection.anchor.username.clone(),
            slot: selection.slot,
            variant: selection.variant,
            blob_id: format!("snsblob-{}", uuid::Uuid::new_v4()),
            chunks: content.bytes.len().div_ceil(CHUNK),
            size: content.bytes.len(),
            sha256: hex::encode(Sha256::digest(&*content.bytes)),
            mime: content.mime.into(),
            file_name: format!(
                "朋友圈-{}-{}.{}",
                selection.anchor.id,
                selection.slot + 1,
                content.extension
            ),
            saved_at: chrono::Utc::now().timestamp(),
            source_md5_matches: content.source_md5_matches,
        };
        let observed = self.new_media_moment_document(account, &content.moment)?;
        let mut docs = content
            .bytes
            .chunks(CHUNK)
            .enumerate()
            .map(|(index, bytes)| {
                lease.check()?;
                self.seal_document_unchecked(
                    account,
                    &format!("{}:{index}", manifest.blob_id),
                    "media_chunk",
                    1,
                    &crate::source_capture::media::MediaChunk {
                        blob_id: manifest.blob_id.clone(),
                        index: index as u64,
                        hex: hex::encode(bytes),
                    },
                )
            })
            .chain(std::iter::once_with(|| {
                lease.check()?;
                self.seal_document_unchecked(account, &manifest.id, "moment_media", 1, &manifest)
            }))
            .chain(observed.into_iter().map(Ok));
        self.repository.library_import(account, &mut docs)?;
        Ok(manifest)
    }
    pub fn moment_media_contents_local(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<(MomentMediaManifest, Zeroizing<Vec<u8>>)> {
        let (_, m) = self.open_document::<MomentMediaManifest>(account, id, "moment_media")?;
        if m.id != id
            || m.size == 0
            || m.size > werc_domain::video::MAX_VIDEO_BYTES
            || m.chunks != m.size.div_ceil(CHUNK)
            || !matches!(m.mime.as_str(), "image/png" | "video/mp4")
        {
            return Err(invalid());
        }
        werc_domain::file::validate_file_name(&m.file_name)?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(m.size));
        for index in 0..m.chunks {
            let (_, part) = self.open_document::<crate::source_capture::media::MediaChunk>(
                account,
                &format!("{}:{index}", m.blob_id),
                "media_chunk",
            )?;
            if part.blob_id != m.blob_id || part.index != index as u64 {
                return Err(invalid());
            }
            let data = Zeroizing::new(hex::decode(&part.hex).map_err(|_| invalid())?);
            if data.len() != CHUNK.min(m.size - bytes.len()) {
                return Err(invalid());
            }
            bytes.extend_from_slice(&data);
        }
        if bytes.len() != m.size || hex::encode(Sha256::digest(&*bytes)) != m.sha256 {
            return Err(invalid());
        }
        Ok((m, bytes))
    }
}
