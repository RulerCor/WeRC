use super::*;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

impl ManagedWindowsWechatProvider {
    pub(super) fn read_voice(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        anchor: &werc_domain::live::LiveVoiceAnchor,
        cancel: &AtomicBool,
    ) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
        let started = Instant::now();
        let check = || {
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::new("QUERY_CANCELED", "语音读取已取消"));
            }
            if started.elapsed() >= Duration::from_secs(60) {
                return Err(AppError::new(
                    "LIVE_VOICE_TIMEOUT",
                    "语音读取已达到时间上限",
                ));
            }
            Ok(())
        };
        check()?;
        if anchor.server_key().is_none() {
            return Err(AppError::new("LIVE_VOICE_ANCHOR_INVALID", "语音定位无效"));
        }
        let (worker, hash) = self
            .live_reader
            .as_ref()
            .ok_or_else(|| AppError::new("LIVE_READER_UNAVAILABLE", "未装配在线读取组件"))?;
        // One shared lifecycle lease throughout; never recursively acquire it.
        let session = self.session.read().map_err(|_| AppError::internal())?;
        let provider = session
            .provider
            .as_ref()
            .ok_or_else(|| AppError::new("NATIVE_NOT_CONNECTED", "当前连接已断开"))?;
        provider.require_open()?;
        let pid = session.pid.ok_or_else(AppError::internal)?;
        let context = provider.resolve_send_context(account, "filehelper")?;
        let path = Path::new(database);
        crate::data_sources::checked(path, false)?;
        validate_live_source(path, &context.sender_id)?;
        let message_pin = crate::live_source::LiveSourcePin::open(path)?;
        message_pin.require(source_fingerprint)?;
        let root = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or_else(AppError::internal)?;
        let catalog = crate::data_sources::inspect_root(root)?;
        if catalog.sources.len() != 1 {
            return Err(AppError::new(
                "LIVE_SOURCE_IDENTITY_MISMATCH",
                "媒体来源目录不唯一",
            ));
        }
        let source = &catalog.sources[0];
        if std::fs::canonicalize(&source.account_directory).ok()
            != Some(std::fs::canonicalize(root).map_err(|_| AppError::internal())?)
        {
            return Err(AppError::new(
                "LIVE_SOURCE_IDENTITY_MISMATCH",
                "媒体来源与所选窗口不同",
            ));
        }
        let candidates: Vec<_> = source
            .databases
            .iter()
            .filter(|d| media_name(Path::new(&d.relative_path)))
            .collect();
        if candidates.len() > 8 {
            return Err(AppError::new(
                "LIVE_VOICE_SOURCE_LIMIT",
                "媒体分片超过本次读取上限",
            ));
        }
        let installations = crate::discovery::discover()?.installations;
        let clients: Vec<_> = installations
            .iter()
            .filter(|i| i.process_ids.contains(&pid) && i.sha256 == context.installation_sha256)
            .collect();
        if clients.len() != 1 {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "当前微信进程身份已变化",
            ));
        }
        let mut found = None;
        for candidate in candidates {
            check()?;
            provider.require_open()?;
            let media_path = root.join(&candidate.relative_path);
            crate::data_sources::checked(&media_path, false)?;
            let _pin = crate::live_source::LiveSourcePin::open(&media_path)?;
            // Temporary verified keys do not mutate the product key revision.
            let key = crate::key_acquisition::acquire(
                clients[0],
                pid,
                media_path.to_str().ok_or_else(AppError::internal)?,
            )?;
            check()?;
            provider.require_open()?;
            let value = crate::live_reader::filehelper_voice_cancellable(
                worker,
                hash,
                &media_path,
                key.key.as_ref(),
                anchor,
                cancel,
            )?;
            if let Some(value) = value {
                if found.is_some() {
                    return Err(AppError::new(
                        "LIVE_VOICE_AMBIGUOUS",
                        "多个媒体来源匹配同一语音，未选择任一项",
                    ));
                }
                found = Some(value);
            }
        }
        check()?;
        provider.require_open()?;
        if provider.resolve_send_context(account, "filehelper")? != context {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "读取期间当前登录身份已变化",
            ));
        }
        Ok(found)
    }
}
fn media_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("media_"))
        .and_then(|n| n.strip_suffix(".db"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
