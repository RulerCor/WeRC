use crate::Application;
use werc_domain::{AppError, AppResult, live::LiveVoiceAnchor, media::AudioAsset};

impl Application {
    /// Read only the voice selected from an authenticated, current single-source window.
    /// The caller never supplies a server ID, media path, key or SQL query.
    pub fn save_live_voice_local(
        &self,
        account: &str,
        view_id: &str,
        local_id: &str,
        request_id: &str,
    ) -> AppResult<AudioAsset> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一项媒体操作仍在进行"))?;
        let lease = self.source_lease(request_id)?;
        lease.check()?;
        let context = self.checked_send_context(account, "filehelper")?;
        let key_revision = self.repository.key_revision(account)?;
        let (database, binding, source, anchor) = {
            let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
            let result = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let (binding, _, source) = self.live_read_binding(account, &result.database)?;
            if binding != result.binding {
                return Err(stale());
            }
            let window = result.view.recovery_window(&binding, view_id)?;
            let row = window
                .rows
                .iter()
                .find(|r| r.local_id == local_id)
                .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "此窗口中没有指定语音"))?;
            if row
                .message_type
                .parse::<i64>()
                .map_or(true, |t| t & 0xffffffff != 34)
            {
                return Err(AppError::new("RECEIVED_MEDIA_INVALID", "此记录不是语音"));
            }
            let anchor = LiveVoiceAnchor {
                username: None,
                server_id: row.server_id.clone().unwrap_or_default(),
                timestamp: row.timestamp,
            };
            if anchor.server_key().is_none() {
                return Err(AppError::new(
                    "LIVE_VOICE_ANCHOR_INVALID",
                    "此语音没有可验证的服务器定位",
                ));
            }
            (result.database.clone(), binding, source, anchor)
        };
        let silk = self
            .provider
            .read_live_voice(account, &database, &source, &anchor, &lease.cancel)?
            .ok_or_else(|| {
                AppError::new("LIVE_VOICE_NOT_CACHED", "微信本地媒体库中尚无这条语音")
            })?;
        lease.check()?;
        werc_domain::voice::silk_duration(&silk)?;
        let wav = self.provider.decode_received_voice(&silk)?;
        lease.check()?;
        let protected = self.encrypt_audio_local(account, wav.to_vec())?;
        // Retain the view lock through the commit so replacing the window cannot
        // race the final validity check. DB revisions are checked in the transaction.
        let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
        let result = slot
            .as_ref()
            .filter(|r| r.account == account && r.binding == binding && r.database == database)
            .ok_or_else(stale)?;
        result.view.recovery_window(&binding, view_id)?;
        if self.checked_send_context(account, "filehelper")? != context
            || self.live_read_binding(account, &database)?.0 != binding
        {
            return Err(stale());
        }
        lease.check()?;
        self.repository
            .save_live_audio(&protected, context.1, key_revision)?;
        Ok(protected.asset)
    }
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "窗口、来源或身份已变化，请重新读取")
}
