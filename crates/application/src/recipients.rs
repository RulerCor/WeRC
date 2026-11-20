//! Preparing a recipient is local selection, never approval or submission.
use crate::Application;
use serde::Deserialize;
use werc_domain::{AppError, AppResult, outbox::PreparedRecipient};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientSelection {
    pub view_id: String,
    pub multi: bool,
}
pub(crate) type SelectedRecipient = (String, Vec<(String, String)>);
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "请重新打开含有消息的会话，再选择收件人")
}
impl Application {
    pub fn prepared_recipients_local(&self, account: &str) -> AppResult<Vec<PreparedRecipient>> {
        self.checked_send_context(account, "filehelper")?;
        self.provider.prepared_recipients(account)
    }
    pub fn prepare_recipient_local(
        &self,
        account: &str,
        selection: RecipientSelection,
    ) -> AppResult<PreparedRecipient> {
        let _guard = self
            .send_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "发送任务正在处理，请稍后选择收件人"))?;
        self.checked_send_context(account, "filehelper")?;
        let (target, sources) = self.selected_recipient(account, &selection)?;
        self.provider.prepare_recipient(account, &target, &sources)
    }
    pub(crate) fn selected_recipient(
        &self,
        account: &str,
        selection: &RecipientSelection,
    ) -> AppResult<SelectedRecipient> {
        if selection.multi {
            let slot = self
                .multi_conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let result = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let binding =
                self.multi_conversation_binding(account, &result.sources, &result.username)?;
            let (rows, _) = result.view.recovery_window(&binding, &selection.view_id)?;
            if rows.is_empty() {
                return Err(stale());
            }
            let sources = result
                .sources
                .iter()
                .map(|s| (s.database.to_string(), s.fingerprint.clone()))
                .collect::<Vec<_>>();
            Ok((result.username.to_string(), sources))
        } else {
            let slot = self
                .conversation_result
                .lock()
                .map_err(|_| AppError::internal())?;
            let result = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(stale)?;
            let binding = self.conversation_binding(account, &result.database, &result.username)?;
            let window = result.view.recovery_window(&binding, &selection.view_id)?;
            if window.rows.is_empty() {
                return Err(stale());
            }
            let source = self.live_read_binding(account, &result.database)?.2;
            Ok((
                result.username.to_string(),
                vec![(result.database.to_string(), source)],
            ))
        }
    }
}
