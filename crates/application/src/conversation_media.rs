//! Media selected only from a current authenticated conversation window.
use crate::Application;
use serde::{Deserialize, Serialize};
use werc_domain::{
    AppError, AppResult,
    file::{FileContents, ProtectedFile},
    live::{LiveMessage, LiveVoiceAnchor},
    media::ProtectedAsset,
    receive::ReceivedText,
    video::{ProtectedVideo, VideoBinding},
};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMediaKind {
    Image,
    Voice,
    Video,
    File,
    Emoticon,
}
#[derive(Serialize)]
pub struct ConversationMediaHint {
    pub local_id: String,
    pub source: Option<String>,
    pub kind: ConversationMediaKind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationMediaSelection {
    pub view_id: String,
    pub local_id: String,
    /// Required for merged views; absent only for single-source views.
    pub source: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationMediaRef {
    pub kind: ConversationMediaKind,
    pub id: String,
    pub proof: String,
}
#[derive(Serialize)]
pub struct ConversationMediaAsset {
    pub reference: ConversationMediaRef,
    pub sha256: String,
    pub size: usize,
}
pub struct ConversationMediaContents {
    pub file: FileContents,
    pub mime: &'static str,
}
pub(crate) struct Selected {
    pub(crate) database: Zeroizing<String>,
    pub(crate) source: String,
    pub(crate) binding: String,
    pub(crate) username: Zeroizing<String>,
    pub(crate) row: LiveMessage,
}
enum Protected {
    File(ProtectedFile),
    Voice(ProtectedAsset),
    Video(ProtectedVideo),
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "会话窗口、来源或身份已变化，请重新打开")
}
fn invalid() -> AppError {
    AppError::new("RECEIVED_MEDIA_INVALID", "此消息没有可读取的受支持媒体")
}
fn message(row: &LiveMessage) -> AppResult<ReceivedText> {
    Ok(ReceivedText {
        sequence: 1,
        native_message_id: row.local_id.clone(),
        text: String::new(),
        message_type: (row.message_type.parse::<i64>().map_err(|_| invalid())? & 0xffffffff) as u32,
        metadata: row.metadata_body().map(str::to_owned),
    })
}
fn kind(row: &LiveMessage) -> AppResult<ConversationMediaKind> {
    use ConversationMediaKind::*;
    let m = message(row)?;
    match m.message_type {
        3 => {
            werc_domain::received_media::image_descriptor(&m)?;
            Ok(Image)
        }
        34 => {
            if (LiveVoiceAnchor {
                username: None,
                server_id: row.server_id.clone().unwrap_or_default(),
                timestamp: row.timestamp,
            })
            .server_key()
            .is_none()
            {
                return Err(invalid());
            }
            Ok(Voice)
        }
        43 => {
            werc_domain::received_media::video_descriptor(&m)?;
            Ok(Video)
        }
        47 => {
            werc_domain::received_emoticon::descriptor(&m)?;
            Ok(Emoticon)
        }
        49 => {
            if werc_domain::received_emoticon::descriptor(&m).is_ok() {
                Ok(Emoticon)
            } else {
                werc_domain::received_media::file_descriptor(&m)?;
                Ok(File)
            }
        }
        _ => Err(invalid()),
    }
}
pub(crate) fn hint(row: &LiveMessage, source: Option<&str>) -> Option<ConversationMediaHint> {
    Some(ConversationMediaHint {
        local_id: row.local_id.clone(),
        source: source.map(str::to_owned),
        kind: kind(row).ok()?,
    })
}
impl Application {
    // The callback runs while the selected view lock is retained. Both the first
    // selection and the final encrypted commit use this same authentication path.
    pub(crate) fn with_conversation_message<T>(
        &self,
        account: &str,
        selection: &ConversationMediaSelection,
        apply: impl FnOnce(Selected) -> AppResult<T>,
    ) -> AppResult<T> {
        if let Some(tag) = selection.source.as_deref() {
            let slot = self
                .multi_conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let r = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let binding = self.multi_conversation_binding(account, &r.sources, &r.username)?;
            let (rows, _) = r.view.recovery_window(&binding, &selection.view_id)?;
            let row = rows
                .into_iter()
                .find(|r| r.source == tag && r.message.local_id == selection.local_id)
                .ok_or_else(invalid)?;
            let source = r.sources.iter().find(|s| s.tag == tag).ok_or_else(stale)?;
            apply(Selected {
                database: source.database.clone(),
                source: source.fingerprint.clone(),
                binding,
                username: r.username.clone(),
                row: row.message,
            })
        } else {
            let slot = self
                .conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let r = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let binding = self.conversation_binding(account, &r.database, &r.username)?;
            let window = r.view.recovery_window(&binding, &selection.view_id)?;
            let row = window
                .rows
                .iter()
                .find(|r| r.local_id == selection.local_id)
                .ok_or_else(invalid)?
                .clone();
            let source = self.live_read_binding(account, &r.database)?.2;
            apply(Selected {
                database: r.database.clone(),
                source,
                binding,
                username: r.username.clone(),
                row,
            })
        }
    }
    pub fn save_conversation_media_local(
        &self,
        account: &str,
        selection: &ConversationMediaSelection,
        request_id: &str,
    ) -> AppResult<ConversationMediaAsset> {
        self.save_conversation_media_entry_local(account, selection, None, request_id)
    }
    pub fn save_conversation_media_entry_local(
        &self,
        account: &str,
        selection: &ConversationMediaSelection,
        forward_index: Option<usize>,
        request_id: &str,
    ) -> AppResult<ConversationMediaAsset> {
        self.save_conversation_media_path_local(account, selection, forward_index, None, request_id)
    }
    pub fn save_conversation_media_path_local(
        &self,
        account: &str,
        selection: &ConversationMediaSelection,
        forward_index: Option<usize>,
        forward_path: Option<&[usize]>,
        request_id: &str,
    ) -> AppResult<ConversationMediaAsset> {
        let selected = self.with_conversation_message(account, selection, Ok)?;
        self.save_selected_media_local(
            account,
            selected,
            forward_index,
            forward_path,
            request_id,
            false,
            || self.with_conversation_message(account, selection, Ok),
        )
    }
    pub(crate) fn save_selected_media_local(
        &self,
        account: &str,
        selected: Selected,
        forward_index: Option<usize>,
        forward_path: Option<&[usize]>,
        request_id: &str,
        temporary: bool,
        validate: impl Fn() -> AppResult<Selected>,
    ) -> AppResult<ConversationMediaAsset> {
        if forward_index.is_some() && forward_path.is_some() {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "只能选择一个合并记录条目",
            ));
        }
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一项媒体操作仍在进行"))?;
        let lease = self.browse_lease(request_id)?;
        lease.check()?;
        let context = self.checked_send_context(account, "filehelper")?;
        let key_revision = self.repository.key_revision(account)?;
        let legacy_forward = forward_index
            .map(|index| {
                if message(&selected.row)?.message_type != 49 {
                    return Err(invalid());
                }
                werc_domain::message_card::forward_media::image_descriptor(
                    selected.row.metadata_body().ok_or_else(invalid)?,
                    index,
                )
            })
            .transpose()?;
        let nested_media = forward_path
            .map(|path| {
                if message(&selected.row)?.message_type != 49 {
                    return Err(invalid());
                }
                werc_domain::message_card::forward_media::descriptor(
                    selected.row.metadata_body().ok_or_else(invalid)?,
                    path,
                )
            })
            .transpose()?;
        use werc_domain::message_card::forward_media::ForwardMedia;
        let (forward, forward_file) = match nested_media {
            Some(ForwardMedia::Image(image)) => (Some(image), None),
            Some(ForwardMedia::File(file)) => (None, Some(file)),
            None => (legacy_forward, None),
        };
        let kind = if forward_file.is_some() {
            ConversationMediaKind::File
        } else if forward.is_some() {
            ConversationMediaKind::Image
        } else {
            kind(&selected.row)?
        };
        // Reopening the same selected video reuses its authenticated cache.
        // A missed UI reply must not cause another native download or conflict.
        if kind == ConversationMediaKind::Video && !temporary {
            let id = self.conversation_video_id(account, &selected)?;
            if let Some((asset, proof)) = self.repository.received_video_info(account, &id)? {
                if asset.account_id != account
                    || proof
                        != self.send_digest(account, "conversation-video-link", &(&id, &asset))?
                {
                    return Err(invalid());
                }
                return (|| {
                    let current = validate()?;
                    if current.binding != selected.binding
                        || self.checked_send_context(account, "filehelper")? != context
                    {
                        return Err(stale());
                    }
                    lease.check()?;
                    let proof =
                        self.send_digest(account, "conversation-media-reference-v1", &(kind, &id))?;
                    Ok(ConversationMediaAsset {
                        reference: ConversationMediaRef { kind, id, proof },
                        sha256: asset.sha256,
                        size: asset.size,
                    })
                })();
            }
        }
        let m = message(&selected.row)?;
        let protected = match kind {
            ConversationMediaKind::Voice => {
                let anchor = LiveVoiceAnchor {
                    username: Some(selected.username.to_string()),
                    server_id: selected.row.server_id.clone().unwrap_or_default(),
                    timestamp: selected.row.timestamp,
                };
                let silk = self
                    .provider
                    .read_live_voice(
                        account,
                        &selected.database,
                        &selected.source,
                        &anchor,
                        &lease.cancel,
                    )?
                    .ok_or_else(|| {
                        AppError::new("LIVE_VOICE_NOT_CACHED", "微信本地媒体库中尚无这条语音")
                    })?;
                lease.check()?;
                werc_domain::voice::silk_duration(&silk)?;
                let wav = self.provider.decode_received_voice(&silk)?;
                Protected::Voice(self.encrypt_audio_local(account, wav.to_vec())?)
            }
            ConversationMediaKind::Image => {
                let d = if forward.is_none() {
                    Some(werc_domain::received_media::image_descriptor(&m)?)
                } else {
                    None
                };
                let d = forward
                    .as_ref()
                    .map(|f| &f.media)
                    .or(d.as_ref())
                    .ok_or_else(invalid)?;
                let bytes = if forward.is_some() {
                    self.provider.download_original_image(&context.0, d)?
                } else {
                    self.provider.download_received_image(&context.0, d)?
                };
                lease.check()?;
                if let Some(f) = &forward {
                    f.validate_bytes(&bytes)?;
                }
                let (_, png) = werc_domain::image::canonical_received_png(&bytes)?;
                Protected::File(self.encrypt_file_local(
                    account,
                    "received-image.png",
                    png.to_vec(),
                )?)
            }
            ConversationMediaKind::File => {
                if let Some(d) = &forward_file {
                    let bytes = self.provider.read_live_forward_file(
                        account,
                        &selected.database,
                        &selected.source,
                        d,
                        &lease.cancel,
                    )?;
                    lease.check()?;
                    d.validate_bytes(&bytes)?;
                    Protected::File(self.encrypt_file_local(
                        account,
                        &d.file_name,
                        bytes.to_vec(),
                    )?)
                } else {
                    let d = werc_domain::received_media::file_descriptor(&m)?;
                    let bytes = self.provider.download_received_file(&context.0, &d.media)?;
                    lease.check()?;
                    if bytes.len() as u64 != d.media.declared_size {
                        return Err(invalid());
                    }
                    Protected::File(self.encrypt_file_local(
                        account,
                        &d.file_name,
                        bytes.to_vec(),
                    )?)
                }
            }
            ConversationMediaKind::Emoticon => {
                let d = werc_domain::received_emoticon::descriptor(&m)?;
                let bytes = self.provider.read_live_emoticon(
                    account,
                    &selected.database,
                    &selected.source,
                    &d,
                    &lease.cancel,
                )?;
                lease.check()?;
                d.validate_bytes(&bytes)?;
                if bytes.starts_with(b"wxgf") {
                    self.provider
                        .render_received_emoticon(&bytes, &lease.cancel)?;
                }
                Protected::File(self.encrypt_file_local(
                    account,
                    d.stored_name(&bytes),
                    bytes.to_vec(),
                )?)
            }
            ConversationMediaKind::Video => {
                let d = werc_domain::received_media::video_descriptor(&m)?;
                let bytes = self.provider.download_received_video(&context.0, &d)?;
                lease.check()?;
                Protected::Video(self.encrypt_video_local(account, &bytes)?)
            }
        };
        lease.check()?;
        (|| {
            let current = validate()?;
            if current.binding != selected.binding
                || current.database != selected.database
                || current.source != selected.source
                || current.username != selected.username
                || self.checked_send_context(account, "filehelper")? != context
            {
                return Err(stale());
            }
            lease.check()?;
            let (id, sha256, size) = match &protected {
                Protected::File(p) => {
                    self.repository.save_live_file(p, context.1, key_revision)?;
                    (p.asset.id.clone(), p.asset.sha256.clone(), p.asset.size)
                }
                Protected::Voice(p) => {
                    self.repository
                        .save_live_audio(p, context.1, key_revision)?;
                    (p.asset.id.clone(), p.asset.sha256.clone(), p.asset.size)
                }
                Protected::Video(p) => {
                    // Archive acquisition owns a private temporary reference. It must
                    // never remove the deterministic cache used by open chat previews.
                    let id = if temporary {
                        self.send_digest(
                            account,
                            "archive-video-temporary",
                            &uuid::Uuid::new_v4().to_string(),
                        )?
                    } else {
                        self.conversation_video_id(account, &selected)?
                    };
                    let proof =
                        self.send_digest(account, "conversation-video-link", &(&id, &p.asset))?;
                    self.repository.save_received_video(
                        p,
                        &VideoBinding {
                            observation_id: &id,
                            connection_revision: context.1,
                            key_revision,
                            proof: &proof,
                        },
                    )?;
                    (id, p.asset.sha256.clone(), p.asset.size)
                }
            };
            let proof =
                self.send_digest(account, "conversation-media-reference-v1", &(kind, &id))?;
            Ok(ConversationMediaAsset {
                reference: ConversationMediaRef { kind, id, proof },
                sha256,
                size,
            })
        })()
    }
    fn conversation_video_id(&self, account: &str, selected: &Selected) -> AppResult<String> {
        self.send_digest(
            account,
            "conversation-video-selection-v1",
            &(
                &selected.source,
                &*selected.username,
                &selected.row.local_id,
                &selected.row.server_id,
                selected.row.timestamp,
            ),
        )
    }
    fn check_conversation_media_ref(
        &self,
        account: &str,
        r: &ConversationMediaRef,
    ) -> AppResult<()> {
        self.ensure_account(account)?;
        if r.id.len() > 64
            || r.proof
                != self.send_digest(account, "conversation-media-reference-v1", &(r.kind, &r.id))?
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn conversation_media_content_local(
        &self,
        account: &str,
        r: &ConversationMediaRef,
    ) -> AppResult<ConversationMediaContents> {
        self.check_conversation_media_ref(account, r)?;
        let (file, mime) = match r.kind {
            ConversationMediaKind::Voice => (
                FileContents {
                    file_name: "received-voice.wav".into(),
                    bytes: self.audio_preview_local(account, &r.id)?,
                },
                "audio/wav",
            ),
            ConversationMediaKind::Image => (self.decrypt_file_local(account, &r.id)?, "image/png"),
            ConversationMediaKind::File => (
                self.decrypt_file_local(account, &r.id)?,
                "application/octet-stream",
            ),
            ConversationMediaKind::Emoticon => {
                let f = self.emoticon_contents_local(account, &r.id)?;
                let mime = if f.file_name.ends_with(".png") {
                    "image/png"
                } else {
                    "image/gif"
                };
                (f, mime)
            }
            ConversationMediaKind::Video => {
                let (asset, proof) = self
                    .repository
                    .received_video_info(account, &r.id)?
                    .ok_or_else(|| AppError::new("ASSET_NOT_FOUND", "视频未保存或已过期"))?;
                if asset.account_id != account
                    || proof
                        != self.send_digest(account, "conversation-video-link", &(&r.id, &asset))?
                {
                    return Err(invalid());
                }
                let protected = self.repository.received_video(account, &r.id)?;
                (
                    FileContents {
                        file_name: "received-video.mp4".into(),
                        bytes: self.decrypt_video_local(account, &asset, protected)?,
                    },
                    "video/mp4",
                )
            }
        };
        Ok(ConversationMediaContents { file, mime })
    }
    pub fn delete_conversation_media_local(
        &self,
        account: &str,
        r: &ConversationMediaRef,
    ) -> AppResult<()> {
        self.check_conversation_media_ref(account, r)?;
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一项媒体操作仍在进行"))?;
        match r.kind {
            ConversationMediaKind::Voice => self.delete_audio_local(account, &r.id),
            ConversationMediaKind::Video => self.repository.delete_received_video(account, &r.id),
            _ => self.delete_file_local(account, &r.id),
        }
    }
}
