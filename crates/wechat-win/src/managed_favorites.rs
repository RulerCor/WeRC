use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use werc_domain::favorites::{FavoriteSelection, FavoriteWindow};

impl ManagedWindowsWechatProvider {
    pub(super) fn read_favorites(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        selection: &FavoriteSelection,
        expected_fingerprint: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, FavoriteWindow)> {
        let check = || {
            if cancel.load(Ordering::SeqCst) {
                Err(AppError::new("QUERY_CANCELED", "微信收藏读取已取消"))
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
        // No caller-supplied favorites path, SQL, or guessed account name.
        let favorites_path = storage.join("favorite").join("favorite.db");
        crate::data_sources::checked(&favorites_path, false)?;
        let favorites_pin = crate::live_source::LiveSourcePin::open(&favorites_path)?;
        let fingerprint = favorites_pin.fingerprint().to_owned();
        if let Some(expected) = expected_fingerprint {
            favorites_pin.require(expected)?;
        }
        check()?;
        let key = session.auxiliary_keys.acquire(
            account,
            &context,
            pid,
            &favorites_path,
            &fingerprint,
        )?;
        check()?;
        provider.require_open()?;
        let window = crate::live_reader::favorites_cancellable(
            worker,
            hash,
            &favorites_path,
            key.as_ref(),
            selection,
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
