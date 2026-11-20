use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use werc_domain::live::{LiveSessionContinuation, LiveSessionWindow};

impl ManagedWindowsWechatProvider {
    pub(super) fn read_sessions(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&LiveSessionContinuation>,
        search_query: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, LiveSessionWindow)> {
        let check = || {
            if cancel.load(Ordering::SeqCst) {
                Err(AppError::new("QUERY_CANCELED", "会话读取已取消"))
            } else {
                Ok(())
            }
        };
        check()?;
        let (worker, hash) = self
            .live_reader
            .as_ref()
            .ok_or_else(|| AppError::new("LIVE_READER_UNAVAILABLE", "未装配在线读取组件"))?;
        let session = self.session.read().map_err(|_| AppError::internal())?;
        let provider = session
            .provider
            .as_ref()
            .ok_or_else(|| AppError::new("NATIVE_NOT_CONNECTED", "当前连接已断开"))?;
        provider.require_open()?;
        let context = provider.resolve_send_context(account, "filehelper")?;
        let pid = session.pid.ok_or_else(AppError::internal)?;
        let path = Path::new(database);
        crate::data_sources::checked(path, false)?;
        validate_live_source(path, &context.sender_id)?;
        let message_pin = crate::live_source::LiveSourcePin::open(path)?;
        message_pin.require(source_fingerprint)?;
        let storage = path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(AppError::internal)?;
        // Fixed relative layout under the already authenticated account root.
        // No caller-supplied session path, SQL, or guessed account name.
        let session_path = storage.join("session").join("session.db");
        crate::data_sources::checked(&session_path, false)?;
        let session_pin = crate::live_source::LiveSourcePin::open(&session_path)?;
        let fingerprint = session_pin.fingerprint().to_owned();
        if let Some(after) = after {
            session_pin.require(&after.source_fingerprint)?;
        }
        check()?;
        let key =
            session
                .auxiliary_keys
                .acquire(account, &context, pid, &session_path, &fingerprint)?;
        check()?;
        provider.require_open()?;
        let window = crate::live_reader::sessions_filtered_cancellable(
            worker,
            hash,
            &session_path,
            key.as_ref(),
            after.map(|a| &a.anchor),
            search_query,
            cancel,
        )?;
        check()?;
        provider.require_open()?;
        if provider.resolve_send_context(account, "filehelper")? != context {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "读取期间登录身份已变化",
            ));
        }
        Ok((fingerprint, window))
    }
}
