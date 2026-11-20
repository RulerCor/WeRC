use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use werc_domain::special_records::{RecordSelection, RecordWindow};

impl ManagedWindowsWechatProvider {
    pub(super) fn read_special_records(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        selection: &RecordSelection,
        expected_fingerprint: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, RecordWindow)> {
        let check = || {
            if cancel.load(Ordering::SeqCst) {
                Err(AppError::new("QUERY_CANCELED", "专项记录读取已取消"))
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
        let records_path = storage.join("general").join("general.db");
        crate::data_sources::checked(&records_path, false)?;
        let records_pin = crate::live_source::LiveSourcePin::open(&records_path)?;
        let fingerprint = records_pin.fingerprint().to_owned();
        if let Some(expected) = expected_fingerprint {
            records_pin.require(expected)?;
        }
        check()?;
        let key =
            session
                .auxiliary_keys
                .acquire(account, &context, pid, &records_path, &fingerprint)?;
        check()?;
        provider.require_open()?;
        let window = crate::live_reader::special_records_cancellable(
            worker,
            hash,
            &records_path,
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
