use crate::Application;
use werc_domain::{AppError, AppResult, file::FileAsset, receive::ReceivedText};

impl Application {
    /// Read only the emoticon selected from an authenticated, current single-source window.
    /// The caller never supplies a server ID, media path, key or SQL query.
    pub fn save_live_emoticon_local(
        &self,
        account: &str,
        view_id: &str,
        local_id: &str,
        request_id: &str,
    ) -> AppResult<FileAsset> {
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
                .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "此窗口中没有指定表情"))?;
            if row
                .message_type
                .parse::<i64>()
                .map_or(true, |t| !matches!(t & 0xffffffff, 47 | 49))
            {
                return Err(AppError::new("RECEIVED_MEDIA_INVALID", "此记录不是表情"));
            }
            let anchor = werc_domain::received_emoticon::descriptor(&ReceivedText {
                sequence: 1,
                native_message_id: row.local_id.clone(),
                text: String::new(),
                message_type: (row.message_type.parse::<i64>().map_err(|_| stale())? & 0xffffffff)
                    as u32,
                metadata: row.metadata_body().map(str::to_owned),
            })?;
            (result.database.clone(), binding, source, anchor)
        };
        let bytes = self.provider.read_live_emoticon(
            account,
            &database,
            &source,
            &anchor,
            &lease.cancel,
        )?;
        lease.check()?;
        anchor.validate_bytes(&bytes)?;
        if bytes.starts_with(b"wxgf") {
            self.provider
                .render_received_emoticon(&bytes, &lease.cancel)?;
        }
        let protected =
            self.encrypt_file_local(account, anchor.stored_name(&bytes), bytes.to_vec())?;
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
            .save_live_file(&protected, context.1, key_revision)?;
        Ok(protected.asset)
    }
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "窗口、来源或身份已变化，请重新读取")
}
