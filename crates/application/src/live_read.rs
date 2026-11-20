//! Explicit local bounded live reads. No import, outbox mutation or gap completion.
use crate::{
    Application,
    live_view::{LiveDirection, LivePage, LiveReadView},
};
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult};
use zeroize::Zeroizing;

pub(crate) struct LiveResult {
    pub(crate) account: String,
    pub(crate) database: Zeroizing<String>,
    pub(crate) binding: String,
    pub(crate) view: LiveReadView,
}
impl Application {
    pub fn bind_live_source_local(
        &self,
        account: &str,
        database: &str,
        confirmed: bool,
    ) -> AppResult<()> {
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "需要确认将所选来源用于当前微信只读查询",
            ));
        }
        let (_, revision) = self.checked_send_context(account, "filehelper")?;
        self.provider.validate_live_source(account, database)?;
        let canonical = std::fs::canonicalize(database)
            .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "来源不可访问"))?;
        let root = crate::onboarding::source_root(&canonical)?;
        let fingerprint = hex::encode(Sha256::digest(
            format!("WeRC/setup-source/v1\0{root}").as_bytes(),
        ));
        self.repository
            .bind_current_source(account, revision, &fingerprint)?;
        Ok(())
    }
    pub(crate) fn live_read_binding(
        &self,
        account: &str,
        database: &str,
    ) -> AppResult<(String, [u8; 16], String)> {
        if database.len() > 4096 || database.chars().any(char::is_control) {
            return Err(AppError::new("INVALID_ARGUMENT", "来源路径无效"));
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let canonical = std::fs::canonicalize(database)
            .map_err(|_| AppError::new("SOURCE_NOT_FOUND", "来源不可访问"))?;
        let root = crate::onboarding::source_root(&canonical)?;
        let fingerprint = hex::encode(Sha256::digest(
            format!("WeRC/setup-source/v1\0{root}").as_bytes(),
        ));
        if self.repository.source_owner(&fingerprint)?.as_deref() != Some(account) {
            return Err(AppError::new(
                "SOURCE_CONFLICT",
                "所选来源未绑定到当前微信；没有合并旧资料",
            ));
        }
        let salt = self.provider.database_salt(database)?;
        let source = self.provider.live_source_fingerprint(database)?;
        if source.len() != 64 || !source.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new(
                "LIVE_SOURCE_ID_UNAVAILABLE",
                "来源文件身份无效",
            ));
        }
        let engine = self.provider.live_reader_fingerprint()?;
        if engine.len() != 64 || !engine.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::new("LIVE_COMPONENT_INVALID", "在线引擎指纹无效"));
        }
        let binding = self.send_digest(
            account,
            "live-read-context-v2",
            &(
                context,
                revision,
                canonical.to_string_lossy().to_lowercase(),
                salt,
                &source,
                self.repository.key_revision(account)?,
                engine,
            ),
        )?;
        Ok((binding, salt, source))
    }
    pub fn prepare_live_filehelper_local(
        &self,
        account: &str,
        database: &str,
        request_id: &str,
        confirmed: bool,
    ) -> AppResult<LivePage> {
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "需要确认只读所选当前来源",
            ));
        }
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.exclusive(|| lease.check())?;
        self.prepare_live_inner(account, database, &lease, None)
    }
    pub fn continue_live_filehelper_local(
        &self,
        account: &str,
        cursor: &str,
        request_id: &str,
    ) -> AppResult<LivePage> {
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.exclusive(|| lease.check())?;
        let (database, binding, anchor) = {
            let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
            let result = slot
                .as_ref()
                .filter(|r| r.account == account)
                .ok_or_else(|| AppError::new("CURSOR_STALE", "在线窗口已失效"))?;
            let binding = self.live_read_binding(account, &result.database)?.0;
            if binding != result.binding {
                return Err(AppError::new("CURSOR_STALE", "来源或当前身份已变化"));
            }
            let anchor = result.view.continuation_anchor(&binding, cursor)?;
            (result.database.clone(), binding, anchor)
        };
        self.prepare_live_inner(account, &database, &lease, Some((&anchor, &binding)))
    }
    fn prepare_live_inner(
        &self,
        account: &str,
        database: &str,
        lease: &crate::history::QueryLease<'_>,
        before: Option<(&werc_domain::live::LiveAnchor, &str)>,
    ) -> AppResult<LivePage> {
        lease.check()?;
        let (context, salt, source) = self.live_read_binding(account, database)?;
        if before.is_some_and(|(_, expected)| expected != context) {
            return Err(AppError::new("CURSOR_STALE", "来源或当前身份已变化"));
        }
        let keys = self.live_keys_local(account)?;
        let key = crate::keyring::key_for(&keys, &salt)?;
        let window = self.provider.read_live_filehelper(
            account,
            database,
            &key,
            &source,
            before.map(|(a, _)| a),
            &lease.cancel,
        )?;
        lease.check()?;
        if self.live_read_binding(account, database)?.0 != context {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "读取期间身份、来源或组件已变化",
            ));
        }
        self.enroll_backfill_source(account, database)?;
        let view = LiveReadView::new(&context, window)?;
        let page = view.page(&context, None, 60)?;
        let mut slot = self.live_result.lock().map_err(|_| AppError::internal())?;
        lease.check()?;
        self.multi_live_result
            .lock()
            .map_err(|_| AppError::internal())?
            .take();
        *slot = Some(LiveResult {
            account: account.into(),
            database: Zeroizing::new(database.into()),
            binding: context,
            view,
        });
        Ok(page)
    }
    pub(crate) fn live_keys_local(&self, account: &str) -> AppResult<Zeroizing<Vec<u8>>> {
        let binding = self
            .bindings_local()?
            .into_iter()
            .find(|b| b.account_id == account)
            .ok_or_else(|| AppError::new("CONNECTION_REQUIRED", "当前来源未绑定"))?;
        let (protected, _) = self.repository.protected_key(account)?.ok_or_else(|| {
            AppError::new("KEY_REQUIRED", "请先为所选库获取并保护密钥，无需导入历史")
        })?;
        let keys = Zeroizing::new(self.provider.unprotect_key(
            &format!("WeRC/key/v1/{account}/{}", binding.installation.sha256),
            &protected,
        )?);
        Ok(keys)
    }
    pub fn live_filehelper_page_local(
        &self,
        account: &str,
        cursor: Option<(&str, LiveDirection)>,
        limit: usize,
    ) -> AppResult<LivePage> {
        let mut slot = self.live_result.lock().map_err(|_| AppError::internal())?;
        let result = slot
            .as_ref()
            .ok_or_else(|| AppError::new("CURSOR_STALE", "请重新读取在线窗口"))?;
        if result.account != account || result.view.expired() {
            slot.take();
            return Err(AppError::new(
                "CURSOR_STALE",
                "在线窗口已过期或当前账号已变化",
            ));
        }
        let binding = match self.live_read_binding(account, &result.database) {
            Ok((b, _, _)) => b,
            Err(e) => {
                slot.take();
                return Err(e);
            }
        };
        if binding != result.binding {
            slot.take();
            return Err(AppError::new("CURSOR_STALE", "当前身份、来源或引擎已变化"));
        }
        result.view.page(&binding, cursor, limit)
    }
    pub fn clear_live_filehelper_local(&self) -> AppResult<()> {
        self.multi_conversation_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.multi_conversation_result
            .lock()
            .map_err(|_| AppError::internal())?
            .take();
        self.conversation_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.conversation_result
            .lock()
            .map_err(|_| AppError::internal())?
            .take();
        self.live_result
            .lock()
            .map_err(|_| AppError::internal())?
            .take();
        self.multi_live_result
            .lock()
            .map_err(|_| AppError::internal())?
            .take();
        Ok(())
    }
}
