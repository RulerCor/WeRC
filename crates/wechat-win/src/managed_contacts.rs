use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use werc_domain::live::{LiveContactContinuation, LiveContactWindow};

impl ManagedWindowsWechatProvider {
    pub(super) fn read_contacts(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&LiveContactContinuation>,
        search_query: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, LiveContactWindow)> {
        self.with_contact_cache(
            account,
            database,
            source_fingerprint,
            after.map(|a| a.source_fingerprint.as_str()),
            cancel,
            |worker, hash, path, key| {
                crate::live_reader::contacts_filtered_cancellable(
                    worker,
                    hash,
                    path,
                    key,
                    after.map(|a| &a.anchor),
                    search_query,
                    cancel,
                )
            },
        )
    }
    pub(super) fn read_group_members(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        selection: &werc_domain::group::LiveGroupSelection,
        expected_fingerprint: Option<&str>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, werc_domain::group::LiveGroupWindow)> {
        self.with_contact_cache(
            account,
            database,
            source_fingerprint,
            expected_fingerprint,
            cancel,
            |worker, hash, path, key| {
                crate::live_reader::group_members_cancellable(
                    worker, hash, path, key, selection, cancel,
                )
            },
        )
    }
    fn with_contact_cache<T>(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        expected_fingerprint: Option<&str>,
        cancel: &AtomicBool,
        read: impl FnOnce(&Path, &str, &Path, &[u8]) -> AppResult<T>,
    ) -> AppResult<(String, T)> {
        let check = || {
            if cancel.load(Ordering::SeqCst) {
                Err(AppError::new("QUERY_CANCELED", "联系人读取已取消"))
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
        // No caller-supplied contact path, SQL, or guessed account name.
        let contact_path = storage.join("contact").join("contact.db");
        crate::data_sources::checked(&contact_path, false)?;
        let contact_pin = crate::live_source::LiveSourcePin::open(&contact_path)?;
        let fingerprint = contact_pin.fingerprint().to_owned();
        if let Some(expected) = expected_fingerprint {
            contact_pin.require(expected)?;
        }
        check()?;
        let key =
            session
                .auxiliary_keys
                .acquire(account, &context, pid, &contact_path, &fingerprint)?;
        check()?;
        provider.require_open()?;
        let window = read(worker, hash, &contact_path, key.as_ref())?;
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
