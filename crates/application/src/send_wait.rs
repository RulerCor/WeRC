use crate::Application;
use werc_domain::{AppError, AppResult, Principal, outbox::SendStatus};

impl Application {
    /// Continue the response for a text draft created by this token. This does
    /// not grant operations:read, expose payloads, or permit approval/submission.
    pub fn created_text_send_status(
        &self,
        principal: &Principal,
        account: &str,
        id: &str,
    ) -> AppResult<SendStatus> {
        principal.require("messages:send")?;
        principal.require("operations:create")?;
        principal.require_account(account)?;
        let status = self.repository.send_status(account, id)?;
        if status.metadata.creator_id != principal.id || status.metadata.schema_version != 1 {
            return Err(AppError::new("FORBIDDEN", "只能等待本令牌创建的文字草稿"));
        }
        Ok(status)
    }
}
