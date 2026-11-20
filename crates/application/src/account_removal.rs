//! Explicit account removal must not race an active operation or leave a native session alive.
use crate::{Application, article_jobs::ArticlePlan, export_plans::ExportPlan};
use werc_domain::{AppError, AppResult};

fn busy() -> AppError {
    AppError::new(
        "ACCOUNT_BUSY",
        "此配置仍有操作正在运行，请先完成或取消后再移除",
    )
}
impl Application {
    pub fn remove_account_local(&self, account: &str) -> AppResult<()> {
        self.ensure_account(account)?;
        // These are the existing operation locks. New work cannot pass its
        // account/connection validation after the repository deletion commits.
        let _setup = self.setup_guard.try_lock().map_err(|_| busy())?;
        let _history = self.history_guard.try_lock().map_err(|_| busy())?;
        let _send = self.send_guard.try_lock().map_err(|_| busy())?;
        let _receive = self.receive_guard.try_lock().map_err(|_| busy())?;
        let _backfill = self.backfill_guard.try_lock().map_err(|_| busy())?;
        let _preservation = self
            .message_preservation_guard
            .try_lock()
            .map_err(|_| busy())?;
        let _moments = self
            .moment_preservation_guard
            .try_lock()
            .map_err(|_| busy())?;
        let _media = self.received_media_guard.try_lock().map_err(|_| busy())?;
        let jobs = self.browse_jobs.lock().map_err(|_| AppError::internal())?;
        if !jobs.is_empty() || self.setup_active.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(busy());
        }
        for kind in ["export_plan", "article_plan"] {
            let mut cursor = None;
            loop {
                let page = self.repository.library_list(account, kind, cursor, 100)?;
                for header in page.data {
                    let active = if kind == "export_plan" {
                        let (_, plan): (_, ExportPlan) =
                            self.open_document(account, &header.id, kind)?;
                        !plan.deleted && (plan.spec.enabled || plan.active_run.is_some())
                    } else {
                        let (_, plan): (_, ArticlePlan) =
                            self.open_document(account, &header.id, kind)?;
                        !plan.deleted && (plan.spec.enabled || plan.active_run.is_some())
                    };
                    if active {
                        return Err(AppError::new(
                            "ACCOUNT_PLAN_ACTIVE",
                            "请先停用导出和公众号计划，并结束当前运行，再移除本机配置",
                        ));
                    }
                }
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }
        // Refuse before disconnecting so an ordinary pending-task error does
        // not unexpectedly stop the user's live connection.
        if self.repository.has_pending_tasks(account)? {
            return Err(busy());
        }

        // The repository repeats the pending-task check within its deletion
        // transaction, including work queued by another application instance.
        self.provider.disconnect_account(account)?;
        self.clear_live_filehelper_local()?;
        self.repository.remove_account(account)
    }
}
