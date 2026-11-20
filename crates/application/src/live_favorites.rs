//! Authenticated WeChat Favorites browsing. Raw XML never leaves this module.
use crate::{
    Application, conversation_media::ConversationMediaContents, live_search::SearchTicket,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, favorites::*};

#[derive(Serialize)]
pub struct FavoriteItem {
    pub anchor: FavoriteAnchor,
    pub server_id: String,
    pub kind: u32,
    pub source_user: Option<String>,
    pub conversation: Option<String>,
    pub content_status: String,
    pub content: Option<FavoriteContent>,
    pub item_token: String,
}
#[derive(Serialize)]
pub struct FavoritePage {
    pub rows: Vec<FavoriteItem>,
    pub next: Option<String>,
    pub query: FavoriteQuery,
    pub counts: std::collections::BTreeMap<u32, u64>,
    pub scanned: usize,
    pub expires_in_seconds: u64,
    pub mode: &'static str,
    pub cross_page_snapshot: bool,
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "收藏来源、内容或登录状态已变化，请重新读取")
}
fn digest(row: &FavoriteRaw) -> AppResult<String> {
    let bytes = zeroize::Zeroizing::new(serde_json::to_vec(row).map_err(|_| AppError::internal())?);
    Ok(hex::encode(Sha256::digest(&bytes)))
}
fn content_error(code: &'static str) -> AppError {
    AppError::new(code, "此收藏内容或媒体格式当前无法读取")
}
impl Application {
    pub fn live_favorites_local(
        &self,
        account: &str,
        view_id: &str,
        query: Option<FavoriteQuery>,
        cursor: Option<&str>,
        request_id: &str,
    ) -> AppResult<FavoritePage> {
        if query.is_some() == cursor.is_some() || query.as_ref().is_some_and(|q| !q.valid()) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "首次读取需要筛选条件，继续读取只需游标",
            ));
        }
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        lease.check()?;
        let (binding, source, expected, selection) =
            self.with_search_parent(account, view_id, |binding, parent, sources, _| {
                let (expected, selection) = if let Some(c) = cursor {
                    match parent.open_search(binding, view_id, c)? {
                        SearchTicket::Favorites {
                            fingerprint,
                            selection,
                        } => (Some(fingerprint), selection),
                        _ => return Err(stale()),
                    }
                } else {
                    (
                        None,
                        FavoriteSelection {
                            query: query.ok_or_else(stale)?,
                            after: None,
                            exact: false,
                        },
                    )
                };
                if !selection.valid() || selection.exact {
                    return Err(stale());
                }
                Ok((
                    binding.to_owned(),
                    sources.first().ok_or_else(stale)?.clone(),
                    expected,
                    selection,
                ))
            })?;
        let (fingerprint, window) = self.provider.read_live_favorites(
            account,
            &source.database,
            &source.fingerprint,
            &selection,
            expected.as_deref(),
            &lease.cancel,
        )?;
        lease.check()?;
        if fingerprint.len() != 64
            || !fingerprint.bytes().all(|v| v.is_ascii_hexdigit())
            || expected.as_ref().is_some_and(|v| v != &fingerprint)
            || !window.valid(&selection)
        {
            return Err(stale());
        }
        if zeroize::Zeroizing::new(serde_json::to_vec(&window).map_err(|_| AppError::internal())?)
            .len()
            > MAX_BYTES
        {
            return Err(content_error("FAVORITE_WINDOW_LIMIT"));
        }
        self.with_search_parent(account, view_id, |current, parent, sources, _| {
            if current != binding
                || sources
                    .first()
                    .is_none_or(|s| s.fingerprint != source.fingerprint)
            {
                return Err(stale());
            }
            let expires = parent.check(current, view_id)?;
            let next = window
                .next
                .map(|anchor| {
                    parent.seal_search(
                        current,
                        view_id,
                        &SearchTicket::Favorites {
                            fingerprint: fingerprint.clone(),
                            selection: FavoriteSelection {
                                query: selection.query.clone(),
                                after: Some(anchor),
                                exact: false,
                            },
                        },
                    )
                })
                .transpose()?;
            let mut rows = Vec::new();
            for row in window.rows {
                let content = row.display_content();
                let item_token = parent.seal_search(
                    current,
                    view_id,
                    &SearchTicket::FavoriteItem {
                        fingerprint: fingerprint.clone(),
                        anchor: row.anchor.clone(),
                        digest: digest(&row)?,
                    },
                )?;
                rows.push(FavoriteItem {
                    anchor: row.anchor.clone(),
                    server_id: row.server_id.clone(),
                    kind: row.kind,
                    source_user: row.source_user.clone(),
                    conversation: row.conversation.clone(),
                    content_status: if row.xml.is_some() && content.is_none() {
                        "invalid".into()
                    } else {
                        row.content_status.clone()
                    },
                    content,
                    item_token,
                });
            }
            lease.check()?;
            Ok(FavoritePage {
                rows,
                next,
                query: selection.query,
                counts: window.counts,
                scanned: window.scanned,
                expires_in_seconds: expires,
                mode: "local_wechat_favorites",
                cross_page_snapshot: false,
            })
        })
    }
    fn favorite_exact(
        &self,
        account: &str,
        view: &str,
        token: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<FavoriteRaw> {
        let (binding, source, fingerprint, anchor, expected) =
            self.with_search_parent(account, view, |binding, parent, sources, _| {
                match parent.open_search(binding, view, token)? {
                    SearchTicket::FavoriteItem {
                        fingerprint,
                        anchor,
                        digest,
                    } => Ok((
                        binding.to_owned(),
                        sources.first().ok_or_else(stale)?.clone(),
                        fingerprint,
                        anchor,
                        digest,
                    )),
                    _ => Err(stale()),
                }
            })?;
        let selection = FavoriteSelection {
            query: FavoriteQuery::default(),
            after: Some(anchor),
            exact: true,
        };
        let (found, mut window) = self.provider.read_live_favorites(
            account,
            &source.database,
            &source.fingerprint,
            &selection,
            Some(&fingerprint),
            cancel,
        )?;
        if found != fingerprint || !window.valid(&selection) || window.rows.len() != 1 {
            return Err(stale());
        }
        let row = window.rows.pop().ok_or_else(stale)?;
        if digest(&row)? != expected {
            return Err(stale());
        }
        self.with_search_parent(account, view, |current, _, _, _| {
            if current == binding {
                Ok(())
            } else {
                Err(stale())
            }
        })?;
        Ok(row)
    }
    pub fn copy_favorite_local(
        &self,
        account: &str,
        view: &str,
        token: &str,
        request_id: &str,
    ) -> AppResult<crate::library::LibraryItem> {
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        lease.check()?;
        let row = self.favorite_exact(account, view, token, &lease.cancel)?;
        let content = row.display_content().ok_or_else(||content_error("FAVORITE_CONTENT_INVALID"))?;
        let title = if content.title.trim().is_empty() {
            format!("微信收藏 {}", row.anchor.local_id)
        } else {
            content.title.chars().take(120).collect()
        };
        let mut text = content.text.clone();
        for b in content.blocks {
            text.push_str(&format!("\n{}\n{}", b.title, b.text));
        }
        if let Some(url) = content.url {
            text.push_str(&format!("\n{url}"));
        }
        let tags = content
            .tags
            .into_iter()
            .take(32)
            .map(|t| t.chars().take(32).collect())
            .collect();
        lease.check()?;
        self.library_save_local(
            account,
            None,
            "favorite",
            0,
            crate::library::LibraryEntry {
                transcript: None,
                title,
                text,
                tags,
                source: Some(format!(
                    "wechat-favorite:{}:{}",
                    row.anchor.local_id, row.anchor.updated
                )),
            },
        )
    }
    pub fn favorite_media_local(
        &self,
        account: &str,
        view: &str,
        token: &str,
        slot: &[usize],
        request_id: &str,
    ) -> AppResult<ConversationMediaContents> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一个媒体下载仍在进行"))?;
        let lease = self.source_lease(request_id)?;
        lease.check()?;
        let row = self.favorite_exact(account, view, token, &lease.cancel)?;
        let d = favorite_media(row.xml.as_deref().ok_or_else(stale)?, row.kind, slot)
            .map_err(content_error)?;
        let context = self.checked_send_context(account, "filehelper")?;
        let descriptor = werc_domain::received_media::ReceivedImageDescriptor {
            file_key: d.file_key,
            aes_key: d.aes_key,
            declared_size: d.size,
        };
        let bytes = self
            .provider
            .download_favorite_media(&context.0, d.kind, &descriptor)?;
        lease.check()?;
        if bytes.len() > 16 * 1024 * 1024 || (d.kind != 2 && bytes.len() as u64 != d.size) {
            return Err(content_error("FAVORITE_MEDIA_SIZE_MISMATCH"));
        }
        let (bytes, mime, name) = match d.kind {
            3 => {
                werc_domain::voice::silk_duration(&bytes)?;
                (
                    self.provider.decode_received_voice(&bytes)?,
                    "audio/wav",
                    "微信收藏语音.wav".into(),
                )
            }
            2 => (
                werc_domain::image::canonical_received_png(&bytes)?.1,
                "image/png",
                "微信收藏图片.png".into(),
            ),
            4 => {
                werc_domain::video::inspect_mp4(&bytes)?;
                (bytes, "video/mp4", "微信收藏视频.mp4".into())
            }
            _ => (
                bytes,
                "application/octet-stream",
                if d.name.trim().is_empty() {
                    "微信收藏附件.bin".into()
                } else {
                    d.name
                },
            ),
        };
        // Re-read exact item after download; never release bytes for a changed source/login.
        self.favorite_exact(account, view, token, &lease.cancel)?;
        if self.checked_send_context(account, "filehelper")? != context {
            return Err(stale());
        }
        lease.check()?;
        Ok(ConversationMediaContents {
            file: werc_domain::file::FileContents {
                file_name: name,
                bytes,
            },
            mime,
        })
    }
}
