use crate::Application;
use werc_domain::{AppError, AppResult, current::CurrentConnection};

impl Application {
    pub fn current_connection_local(&self) -> AppResult<CurrentConnection> {
        self.repository.current_connection()
    }
    pub fn select_current_connection_local(
        &self,
        account: &str,
        revision: &str,
        confirmed: bool,
    ) -> AppResult<CurrentConnection> {
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "需要确认唯一当前来源；其他旧资料仅作归档",
            ));
        }
        self.setup_pause_local()?;
        self.repository
            .select_current_connection(account, revision)?;
        self.current_connection_local()
    }
    pub(crate) fn ensure_current_connection(&self, account: &str) -> AppResult<()> {
        if self.repository.current_connection()?.account_id.as_deref() != Some(account) {
            return Err(AppError::new(
                "CURRENT_CONNECTION_REQUIRED",
                "此资料仅供归档浏览，不能连接或继续来源操作",
            ));
        }
        Ok(())
    }
}
