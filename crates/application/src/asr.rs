use crate::Application;
use sha2::{Digest, Sha256};
use std::sync::atomic::Ordering;
use werc_domain::{AppError, AppResult, asr::*};
impl Application {
    pub fn asr_status_local(&self) -> AppResult<AsrStatus> {
        self.provider.asr_status()
    }
    pub fn transcribe_local(
        &self,
        account: &str,
        source: &AudioSource,
        model: &str,
        language: &str,
        request_id: &str,
    ) -> AppResult<Transcript> {
        self.ensure_account(account)?;
        let lease = self.browse_lease(request_id)?;
        lease.check()?;
        let wav = match source {
            AudioSource::Archived {
                capture_id,
                page,
                row,
            } => {
                let mut bytes = zeroize::Zeroizing::new(Vec::new());
                let media = self.export_capture_media_local(
                    account,
                    capture_id,
                    *page,
                    *row,
                    &uuid::Uuid::new_v4().to_string(),
                    &mut *bytes,
                )?;
                if media.kind != crate::conversation_media::ConversationMediaKind::Voice {
                    return Err(AppError::new("INVALID_ARGUMENT", "所选归档媒体不是语音"));
                }
                bytes
            }
            AudioSource::Uploaded { asset_id } => self.audio_preview_local(account, asset_id)?,
            AudioSource::Received { position } => {
                self.received_voice_preview_local(account, *position)?
            }
            AudioSource::SavedVoice { file_id } => {
                let file = self.decrypt_file_local(account, file_id)?;
                self.provider.decode_received_voice(&file.bytes)?
            }
        };
        let engine_identity = self.provider.asr_cache_identity(model, &lease.cancel)?;
        let source_json = serde_json::to_string(source).map_err(|_| AppError::internal())?;
        let cache_id = hex::encode(Sha256::digest(
            serde_json::to_vec(&(
                hex::encode(Sha256::digest(&wav)),
                model,
                language,
                &source_json,
                &engine_identity,
            ))
            .map_err(|_| AppError::internal())?,
        ));
        match self.library_item_local(account, &cache_id, "transcript") {
            Ok(item) => {
                if let Some(transcript) = item.entry.transcript {
                    return Ok(transcript);
                }
            }
            Err(e) if e.code == "NOT_FOUND" => {}
            Err(e) => return Err(e),
        }
        let transcript = self
            .provider
            .transcribe_audio(&wav, model, language, &lease.cancel)?;
        if lease.cancel.load(Ordering::SeqCst) {
            return Err(AppError::new("QUERY_CANCELED", "转写已取消"));
        }
        if self.provider.asr_cache_identity(model, &lease.cancel)? != engine_identity {
            return Err(AppError::new(
                "ASR_MODEL_CHANGED",
                "转写期间模型或组件改变，请重新转写",
            ));
        }
        let entry = crate::library::LibraryEntry {
            transcript: Some(transcript.clone()),
            title: format!(
                "语音转写 · {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            ),
            text: transcript.text.clone(),
            tags: vec!["离线转写".into(), model.into()],
            source: Some(source_json),
        };
        crate::library::validate_entry("transcript", &entry)?;
        let d = self.seal_document(account, &cache_id, "transcript", 1, &entry)?;
        match self.repository.library_save(&[d], None) {
            Ok(()) => {}
            Err(e) if e.code == "DOCUMENT_CONFLICT" => {
                return self
                    .library_item_local(account, &cache_id, "transcript")?
                    .entry
                    .transcript
                    .ok_or_else(|| AppError::new("DOCUMENT_CORRUPTED", "已有转写缺少字幕"));
            }
            Err(e) => return Err(e),
        }
        Ok(transcript)
    }
}
