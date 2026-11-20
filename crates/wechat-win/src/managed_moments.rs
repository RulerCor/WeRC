use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use werc_domain::moments::{LiveMomentContinuation, LiveMomentWindow};

impl ManagedWindowsWechatProvider {
    pub(super) fn read_moments(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&LiveMomentContinuation>,
        cancel: &AtomicBool,
    ) -> AppResult<(String, LiveMomentWindow)> {
        self.read_moments_batch(
            account,
            database,
            source_fingerprint,
            after,
            werc_domain::moments::LIVE_MOMENT_MAX_ROWS,
            cancel,
        )
    }
    pub(super) fn read_moments_batch(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&LiveMomentContinuation>,
        limit: usize,
        cancel: &AtomicBool,
    ) -> AppResult<(String, LiveMomentWindow)> {
        let check = || {
            if cancel.load(Ordering::SeqCst) {
                Err(AppError::new("QUERY_CANCELED", "朋友圈读取已取消"))
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
        // No caller-supplied timeline path, SQL, or guessed account name.
        let moments_path = storage.join("sns").join("sns.db");
        crate::data_sources::checked(&moments_path, false)?;
        let moments_pin = crate::live_source::LiveSourcePin::open(&moments_path)?;
        let fingerprint = moments_pin.fingerprint().to_owned();
        if let Some(after) = after {
            moments_pin.require(&after.source_fingerprint)?;
        }
        check()?;
        let key =
            session
                .auxiliary_keys
                .acquire(account, &context, pid, &moments_path, &fingerprint)?;
        check()?;
        provider.require_open()?;
        let window = crate::live_reader::moments_batch_cancellable(
            worker,
            hash,
            &moments_path,
            key.as_ref(),
            after.map(|a| &a.anchor),
            limit,
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

impl ManagedWindowsWechatProvider {
    pub(super) fn moment_media(
        &self,
        account: &str,
        database: &str,
        fingerprint: &str,
        selection: &werc_domain::moments::MomentMediaSelection,
        cancel: &AtomicBool,
    ) -> AppResult<werc_domain::moments::MomentMediaBytes> {
        let (worker, hash) = self
            .live_reader
            .as_ref()
            .ok_or_else(|| AppError::new("LIVE_READER_UNAVAILABLE", "未装配读取组件"))?;
        let session = self.session.read().map_err(|_| AppError::internal())?;
        let provider = session
            .provider
            .as_ref()
            .ok_or_else(|| AppError::new("NATIVE_NOT_CONNECTED", "当前连接已断开"))?;
        let context = provider.resolve_send_context(account, "filehelper")?;
        let path = Path::new(database);
        crate::data_sources::checked(path, false)?;
        validate_live_source(path, &context.sender_id)?;
        let pin = crate::live_source::LiveSourcePin::open(path)?;
        pin.require(fingerprint)?;
        let sns = path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(AppError::internal)?
            .join("sns/sns.db");
        crate::data_sources::checked(&sns, false)?;
        let sns_pin = crate::live_source::LiveSourcePin::open(&sns)?;
        let key = session.auxiliary_keys.acquire(
            account,
            &context,
            session.pid.ok_or_else(AppError::internal)?,
            &sns,
            sns_pin.fingerprint(),
        )?;
        let descriptor = crate::live_reader::moment_media_cancellable(
            worker,
            hash,
            &sns,
            key.as_ref(),
            selection,
            cancel,
        )?;
        let result = crate::sns_media::download(
            &descriptor,
            selection.variant == werc_domain::moments::MomentMediaVariant::Original,
            cancel,
        )?;
        // Exact descriptor must still belong to the same current row and login.
        let again = crate::live_reader::moment_media_cancellable(
            worker,
            hash,
            &sns,
            key.as_ref(),
            selection,
            cancel,
        )?;
        if descriptor.url != again.url
            || descriptor.token != again.token
            || descriptor.seed != again.seed
            || descriptor.md5 != again.md5
            || descriptor.video != again.video
            || provider.resolve_send_context(account, "filehelper")? != context
        {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "下载期间朋友圈来源或身份变化",
            ));
        }
        crate::live_source::LiveSourcePin::open(path)?.require(fingerprint)?;
        crate::live_source::LiveSourcePin::open(&sns)?.require(sns_pin.fingerprint())?;
        Ok(result)
    }
}
