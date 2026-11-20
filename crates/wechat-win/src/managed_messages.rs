//! Explicit desktop lifecycle for the source-built native filehelper channel.
//! Startup never attaches. All existing local readers remain delegated unchanged.
#[path = "managed_special_records.rs"]
mod special_records;
use crate::{WindowsWechatProvider, native_messages::NativeFileHelperProvider};
use std::{
    path::Path,
    sync::{Mutex, RwLock},
};
use werc_application::ports::CapabilityProvider;
use werc_domain::{AppError, AppResult, connection::*, history::*, outbox::*, receive::*};
#[path = "managed_keys.rs"]
mod auxiliary_keys;
#[path = "managed_contacts.rs"]
mod contacts;
#[path = "managed_emoticon.rs"]
mod emoticon;
#[path = "managed_favorites.rs"]
mod favorites;
#[path = "managed_forward_file.rs"]
mod forward_file;
#[path = "managed_moments.rs"]
mod moments;
#[path = "managed_sessions.rs"]
mod sessions;
#[path = "managed_voice.rs"]
mod voice;

#[derive(Clone, serde::Serialize)]
pub struct NativeStatus {
    pub component_available: bool,
    pub connected: bool,
    pub account_id: Option<String>,
    pub error_code: Option<String>,
    pub scope: &'static str,
    pub process_id: Option<u32>,
}
#[derive(Default)]
struct Session {
    auxiliary_keys: auxiliary_keys::AuxiliaryKeys,
    provider: Option<NativeFileHelperProvider>,
    account: Option<String>,
    error: Option<String>,
    ready: bool,
    pid: Option<u32>,
}
#[derive(Default)]
pub struct ManagedWindowsWechatProvider {
    session: RwLock<Session>,
    lifecycle: Mutex<()>,
    live_reader: Option<(std::path::PathBuf, String)>,
    live_window_rows: Option<usize>,
}
impl ManagedWindowsWechatProvider {
    /// Explicit assembly; a nearby file alone never enables live reading.
    pub fn with_live_reader(path: std::path::PathBuf, sha256: String) -> Self {
        Self {
            session: RwLock::new(Session::default()),
            lifecycle: Mutex::new(()),
            live_reader: Some((path, sha256)),
            live_window_rows: None,
        }
    }
    /// A trusted local caller may choose a smaller window resource budget.
    pub fn with_live_window_limit(mut self, rows: usize) -> AppResult<Self> {
        if !(1..=werc_domain::live::LIVE_WINDOW_MAX_ROWS).contains(&rows) {
            return Err(AppError::new("LIVE_WINDOW_LIMIT", "在线窗口预算无效"));
        }
        self.live_window_rows = Some(rows);
        Ok(self)
    }
    pub fn observed_text_task_fields(
        &self,
        context: &SendContext,
        attempt: &str,
    ) -> AppResult<serde_json::Value> {
        self.with(|p| p.observed_text_task_fields(context, attempt))
    }
    pub fn observed_text_message_id(
        &self,
        context: &SendContext,
        attempt: &str,
    ) -> AppResult<(Option<u64>, u32)> {
        self.with(|p| p.observed_text_message_id(context, attempt))
    }
    pub fn status(&self) -> AppResult<NativeStatus> {
        let s = self.session.read().map_err(|_| AppError::internal())?;
        Ok(NativeStatus {
            component_available: option_env!("WERC_MESSAGE_HOST_SHA256")
                .is_some_and(|s| s.len() == 64),
            connected: s.ready,
            account_id: if s.ready { s.account.clone() } else { None },
            error_code: s.error.clone(),
            scope: "prepared_recipient_text_voice_image_file_video_emoticon_send",
            process_id: if s.ready { s.pid } else { None },
        })
    }
    /// Explicit local construction verifier; never exposed to HTTP or Tauri.
    pub fn probe_long_text_to(
        &self,
        target: &str,
        text: &str,
        members: &[String],
    ) -> AppResult<bool> {
        self.with(|p| p.probe_long_text_to(target, text, members))
    }
    pub fn probe_private_image_moment(
        &self,
        text: &str,
        images: &[MomentImage<'_>],
    ) -> AppResult<(u32, u32)> {
        self.with(|p| p.probe_private_image_moment(text, images))
    }
    pub fn probe_moment(
        &self,
        text: &str,
        images: &[MomentImage<'_>],
        visibility: MomentVisibility,
    ) -> AppResult<(u32, u32)> {
        self.with(|p| p.probe_moment(text, images, visibility))
    }
    pub fn probe_video_moment(&self, text: &str, cover: MomentImage<'_>, video: &VideoSendAsset,
        bytes: &[u8], visibility: MomentVisibility) -> AppResult<(u32, u32)> {
        self.with(|p| p.probe_video_moment(text, cover, video, bytes, visibility))
    }
    pub fn probe_moment_action(&self, sns_id: &str) -> AppResult<(u32, u32)> {
        self.with(|p| p.probe_moment_action(sns_id))
    }
    pub fn probe_private_moment(&self, text: &str) -> AppResult<(u32, u32)> {
        self.with(|p| p.probe_private_moment(text))
    }
    pub fn probe_quote_to(
        &self,
        target: &str,
        original_id: u64,
        original_text: &str,
        reply: &str,
    ) -> AppResult<(u32, u32)> {
        self.with(|p| p.probe_quote_to(target, original_id, original_text, reply))
    }
    pub fn probe_mentions_to(
        &self,
        target: &str,
        text: &str,
        members: &[String],
    ) -> AppResult<bool> {
        self.with(|p| p.probe_mentions_to(target, text, members))
    }
    pub fn connect(&self, account: &str, pid: u32, installation: &str) -> AppResult<()> {
        let executable = std::env::current_exe().map_err(|_| AppError::internal())?;
        let host = executable
            .parent()
            .ok_or_else(AppError::internal)?
            .join("werc-message-host.exe");
        self.connect_pinned(
            account,
            &host,
            option_env!("WERC_MESSAGE_HOST_SHA256").unwrap_or(""),
            pid,
            installation,
        )
    }
    /// Explicit trusted local assembly/testing; never exposed as a remote command.
    pub fn connect_pinned(
        &self,
        account: &str,
        host: &Path,
        hash: &str,
        pid: u32,
        installation: &str,
    ) -> AppResult<()> {
        let _lifecycle = self.lifecycle.lock().map_err(|_| AppError::internal())?;
        let mut s = self.session.write().map_err(|_| AppError::internal())?;
        if s.provider.is_some() {
            return Err(AppError::new(
                "NATIVE_ALREADY_CONNECTED",
                "请先断开当前原生通道",
            ));
        }
        match NativeFileHelperProvider::connect(account, host, hash, pid, installation) {
            Ok(p) => {
                s.provider = Some(p);
                s.account = Some(account.into());
                s.pid = Some(pid);
                s.error = None;
                Ok(())
            }
            Err(e) => {
                s.error = Some(e.code.clone());
                Err(e)
            }
        }
    }
    pub fn disconnect(&self, error: Option<&str>) -> AppResult<()> {
        let _lifecycle = self.lifecycle.lock().map_err(|_| AppError::internal())?;
        self.disconnect_session(error)
    }
    /// A failed durable commit must not unload the unacknowledged native queue.
    pub fn receive_failed(&self, account: &str, code: &str) -> AppResult<()> {
        let _lifecycle = self.lifecycle.lock().map_err(|_| AppError::internal())?;
        {
            let mut session = self.session.write().map_err(|_| AppError::internal())?;
            if !session.ready || session.account.as_deref() != Some(account) {
                return Ok(());
            }
            if matches!(code, "INTERNAL_ERROR" | "KEY_PROTECTION_FAILED") {
                session.error = Some(code.into());
                return Ok(());
            }
        }
        self.disconnect_session(Some(code))
    }
    pub fn receive_progressed(&self, account: &str) -> AppResult<()> {
        let mut session = self.session.write().map_err(|_| AppError::internal())?;
        if session.ready && session.account.as_deref() == Some(account) {
            session.error = None;
        }
        Ok(())
    }
    fn disconnect_session(&self, error: Option<&str>) -> AppResult<()> {
        // Signal long downloads while existing calls still hold shared leases.
        // The exclusive lease then waits for those calls and their slot release.
        {
            let s = self.session.read().map_err(|_| AppError::internal())?;
            if let Some(p) = &s.provider {
                p.request_disconnect();
            }
        }
        let mut s = self.session.write().map_err(|_| AppError::internal())?;
        // Drop joins the owned host;
        // the host unhooks and waits for native completions before unloading.
        drop(s.provider.take());
        s.account = None;
        s.pid = None;
        s.auxiliary_keys = Default::default();
        s.ready = false;
        s.error = error.map(str::to_owned);
        Ok(())
    }
    pub fn mark_ready(&self, account: &str) -> AppResult<()> {
        let mut s = self.session.write().map_err(|_| AppError::internal())?;
        if s.provider.is_none() || s.account.as_deref() != Some(account) {
            return Err(AppError::new("NATIVE_CONTEXT_CHANGED", "连接已变化"));
        }
        s.provider
            .as_ref()
            .ok_or_else(AppError::internal)?
            .require_open()?;
        s.ready = true;
        Ok(())
    }
    fn with<T>(&self, f: impl FnOnce(&NativeFileHelperProvider) -> AppResult<T>) -> AppResult<T> {
        let s = self.session.read().map_err(|_| AppError::internal())?;
        let provider = s.provider.as_ref().ok_or_else(|| {
            AppError::new("NATIVE_NOT_CONNECTED", "请在当前微信页显式连接原生通道")
        })?;
        provider.require_open()?;
        f(provider)
    }
}
impl CapabilityProvider for ManagedWindowsWechatProvider {
    fn wechat_settings_read(&self,a:&str)->AppResult<werc_domain::wechat_settings::SettingsSnapshot>{self.with(|p|p.settings_read(a))}
    fn wechat_settings_apply(&self,a:&str,t:&werc_domain::wechat_settings::SettingTarget,e:&str)->AppResult<werc_domain::wechat_settings::SettingResult>{self.with(|p|p.settings_apply(a,t,e))}
    fn disconnect_account(&self, account: &str) -> AppResult<()> {
        let _lifecycle = self.lifecycle.lock().map_err(|_| AppError::internal())?;
        let matches = self.session.read().map_err(|_| AppError::internal())?
            .account.as_deref() == Some(account);
        if matches { self.disconnect_session(None)?; }
        Ok(())
    }
    fn decode_image_cache(&self,sample:&[u8],key:&ImageCacheKey,cancel:&std::sync::atomic::AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{crate::image_cache::decode(sample,key,cancel)}
    fn recover_image_cache_key(&self,account:&str,sample:&[u8],cancel:&std::sync::atomic::AtomicBool)->AppResult<ImageCacheKey>{
        let session=self.session.read().map_err(|_|AppError::internal())?;
        let provider=session.provider.as_ref().ok_or_else(||AppError::new("NATIVE_NOT_CONNECTED","请先连接当前微信"))?;
        provider.require_open()?;let context=provider.resolve_send_context(account,"filehelper")?;
        let pid=session.pid.ok_or_else(AppError::internal)?;
        let installations=crate::discovery::discover()?.installations;
        let choices=installations.iter().filter(|i|i.process_ids.contains(&pid)&&i.sha256==context.installation_sha256).collect::<Vec<_>>();
        if choices.len()!=1{return Err(AppError::new("NATIVE_CONTEXT_CHANGED","微信进程身份已变化"));}
        let key=crate::key_acquisition::acquire_image(choices[0],pid,sample,cancel)?;
        provider.require_open()?;if provider.resolve_send_context(account,"filehelper")?!=context{return Err(AppError::new("CONNECTION_CHANGED","取钥期间微信身份已变化"));}Ok(key)
    }
    fn download_favorite_media(
        &self,
        context: &SendContext,
        kind: u32,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.with(|p| p.download_favorite_media(context, kind, descriptor))
    }

    fn read_live_special_records(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        selection: &werc_domain::special_records::RecordSelection,
        expected_fingerprint: Option<&str>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::special_records::RecordWindow)> {
        self.read_special_records(
            account,
            database,
            source_fingerprint,
            selection,
            expected_fingerprint,
            cancel,
        )
    }
    fn read_live_favorites(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        selection: &werc_domain::favorites::FavoriteSelection,
        expected_fingerprint: Option<&str>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::favorites::FavoriteWindow)> {
        self.read_favorites(
            account,
            database,
            source_fingerprint,
            selection,
            expected_fingerprint,
            cancel,
        )
    }
    fn asr_status(&self) -> AppResult<werc_domain::asr::AsrStatus> {
        crate::asr::status()
    }
    fn asr_cache_identity(
        &self,
        model: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<String> {
        crate::asr::cache_identity(model, cancel)
    }
    fn transcribe_audio(
        &self,
        wav: &[u8],
        model: &str,
        language: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::asr::Transcript> {
        crate::asr::transcribe(wav, model, language, cancel)
    }
    #[cfg(windows)]
    fn render_received_emoticon(
        &self,
        bytes: &[u8],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        crate::emoticon_codec::render(bytes, cancel)
    }

    fn read_live_group_members(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        selection: &werc_domain::group::LiveGroupSelection,
        expected_fingerprint: Option<&str>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::group::LiveGroupWindow)> {
        self.read_group_members(
            account,
            database,
            source_fingerprint,
            selection,
            expected_fingerprint,
            cancel,
        )
    }

    fn read_live_contacts(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&werc_domain::live::LiveContactContinuation>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::live::LiveContactWindow)> {
        self.read_contacts(account, database, source_fingerprint, after, None, cancel)
    }
    fn read_live_contacts_filtered(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&werc_domain::live::LiveContactContinuation>,
        search_query: Option<&str>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::live::LiveContactWindow)> {
        self.read_contacts(
            account,
            database,
            source_fingerprint,
            after,
            search_query,
            cancel,
        )
    }
    fn read_live_sessions(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&werc_domain::live::LiveSessionContinuation>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::live::LiveSessionWindow)> {
        self.read_sessions(account, database, source_fingerprint, after, None, cancel)
    }
    fn read_live_sessions_filtered(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&werc_domain::live::LiveSessionContinuation>,
        search_query: Option<&str>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::live::LiveSessionWindow)> {
        self.read_sessions(
            account,
            database,
            source_fingerprint,
            after,
            search_query,
            cancel,
        )
    }
    fn read_live_moments_batch(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&werc_domain::moments::LiveMomentContinuation>,
        limit: usize,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::moments::LiveMomentWindow)> {
        self.read_moments_batch(account, database, source_fingerprint, after, limit, cancel)
    }
    fn read_moment_media(
        &self,
        account: &str,
        database: &str,
        fingerprint: &str,
        selection: &werc_domain::moments::MomentMediaSelection,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::moments::MomentMediaBytes> {
        self.moment_media(account, database, fingerprint, selection, cancel)
    }
    fn article_history(&self, account:&str, publisher:&str, cursor:Option<&str>, cancel:&std::sync::atomic::AtomicBool)
        ->AppResult<werc_domain::article_history::ArticleHistoryPage> {
        self.with(|p|p.article_history(account,publisher,cursor,cancel))
    }
    fn fetch_article_image(&self,_account:&str,url:&str,cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::articles::ArticleImageBytes>{crate::articles::fetch_image(url,cancel)}
    fn fetch_contact_avatar(&self,_account:&str,url:&str,cancel:&std::sync::atomic::AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{crate::articles::fetch_avatar(url,cancel)}
    fn fetch_article(&self,account:&str,url:&str,cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::articles::Article>{
        match crate::articles::fetch_public(url,cancel) {
            Err(e) if matches!(e.code.as_str(),"ARTICLE_LOGIN_REQUIRED"|"ARTICLE_CONTENT_UNAVAILABLE") => self.with(|p|p.article_body(account,url,cancel)),
            result=>result,
        }
    }
    fn read_live_moments(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&werc_domain::moments::LiveMomentContinuation>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::moments::LiveMomentWindow)> {
        self.read_moments(account, database, source_fingerprint, after, cancel)
    }
    fn download_received_video(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedVideoDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.with(|p| p.download_received_video(context, descriptor))
    }
    fn read_received_emoticon(
        &self,
        account: &str,
        context: &SendContext,
        descriptor: &werc_domain::received_emoticon::ReceivedEmoticonDescriptor,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.read_emoticon_source(account, None, Some(context), descriptor, cancel)
    }
    fn read_live_emoticon(
        &self,
        account: &str,
        database: &str,
        source: &str,
        descriptor: &werc_domain::received_emoticon::ReceivedEmoticonDescriptor,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.read_emoticon(account, database, source, descriptor, cancel)
    }
    fn read_live_forward_file(
        &self,
        account: &str,
        database: &str,
        source: &str,
        descriptor: &werc_domain::message_card::forward_file::ForwardFile,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.read_forward_file(account, database, source, descriptor, cancel)
    }
    fn read_live_voice(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        anchor: &werc_domain::live::LiveVoiceAnchor,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<Option<zeroize::Zeroizing<Vec<u8>>>> {
        self.read_voice(account, database, source_fingerprint, anchor, cancel)
    }
    fn download_received_file(
        &self,
        context: &werc_domain::outbox::SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.with(|p| p.download_received_file(context, descriptor))
    }
    fn download_original_image(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.with(|p| p.download_original_image(context, descriptor))
    }
    fn download_received_image(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.with(|p| p.download_received_image(context, descriptor))
    }
    fn live_source_fingerprint(&self, database: &str) -> AppResult<String> {
        crate::live_source::fingerprint(Path::new(database))
    }
    fn automatic_message_sources(
        &self,
        account: &str,
    ) -> AppResult<werc_application::ports::AutomaticMessageSources> {
        let session = self.session.read().map_err(|_| AppError::internal())?;
        if !session.ready || session.account.as_deref() != Some(account) {
            return Err(AppError::new("NATIVE_NOT_CONNECTED", "当前账号未连接"));
        }
        let provider = session.provider.as_ref().ok_or_else(AppError::internal)?;
        provider.require_open()?;
        let context = provider.resolve_send_context(account, "filehelper")?;
        let mut groups = Vec::new();
        for source in crate::data_sources::discover()?.sources {
            let mut paths = Vec::new();
            for file in source.databases {
                if !file.wal_present {
                    continue;
                }
                let path = Path::new(&source.account_directory).join(&file.relative_path);
                if validate_live_source(&path, &context.sender_id).is_ok() {
                    crate::data_sources::checked(&path, false)?;
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .ok_or_else(AppError::internal)?;
                    if !path.with_file_name(format!("{name}-shm")).is_file() {
                        continue;
                    }
                    paths.push(path.to_str().ok_or_else(AppError::internal)?.to_owned());
                }
            }
            if !paths.is_empty() {
                groups.push(paths)
            }
        }
        if provider.resolve_send_context(account, "filehelper")? != context {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "发现来源期间登录身份变化",
            ));
        }
        let mut databases: Vec<String> = groups.into_iter().flatten().collect();
        databases.sort();
        if databases.len() > 512 {
            return Err(AppError::new(
                "SOURCE_LIMIT_EXCEEDED",
                "消息分片超过当前读取上限",
            ));
        }
        Ok(werc_application::ports::AutomaticMessageSources {
            process_id: session.pid.ok_or_else(AppError::internal)?,
            databases,
        })
    }
    fn validate_live_source(&self, account: &str, database: &str) -> AppResult<()> {
        self.with(|p| {
            let context = p.resolve_send_context(account, "filehelper")?;
            let path = Path::new(database);
            crate::data_sources::checked(path, false)?;
            validate_live_source(path, &context.sender_id)
        })
    }
    fn live_reader_fingerprint(&self) -> AppResult<String> {
        let (path, hash) = self.live_reader.as_ref().ok_or_else(|| {
            AppError::new("LIVE_READER_UNAVAILABLE", "本次构建未装配在线读取组件")
        })?;
        crate::live_reader::verify_component(path, hash)?;
        Ok(hash.clone())
    }
    fn read_live_filehelper(
        &self,
        account: &str,
        database: &str,
        key: &[u8],
        source_fingerprint: &str,
        before: Option<&werc_domain::live::LiveAnchor>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveWindow> {
        let (worker, hash) = self.live_reader.as_ref().ok_or_else(|| {
            AppError::new("LIVE_READER_UNAVAILABLE", "本次构建未装配在线读取组件")
        })?;
        self.with(|p| {
            let context = p.resolve_send_context(account, "filehelper")?;
            let path = Path::new(database);
            crate::data_sources::checked(path, false)?;
            validate_live_source(path, &context.sender_id)?;
            let source_pin = crate::live_source::LiveSourcePin::open(path)?;
            source_pin.require(source_fingerprint)?;
            let window = crate::live_reader::filehelper_window_before_cancellable(
                worker,
                hash,
                path,
                key,
                self.live_window_rows
                    .unwrap_or(werc_domain::live::LIVE_WINDOW_MAX_ROWS),
                before,
                cancel,
            )?;
            if p.resolve_send_context(account, "filehelper")? != context {
                return Err(AppError::new(
                    "NATIVE_CONTEXT_CHANGED",
                    "读取期间当前登录身份已变化",
                ));
            }
            Ok(window)
        })
    }
    fn read_live_calendar(
        &self,
        account: &str,
        database: &str,
        key: &[u8],
        source_fingerprint: &str,
        selection: &werc_domain::live::LiveCalendarSelection,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveCalendarWindow> {
        let (worker, hash) = self.live_reader.as_ref().ok_or_else(|| {
            AppError::new("LIVE_READER_UNAVAILABLE", "本次构建未装配在线读取组件")
        })?;
        self.with(|p| {
            let context = p.resolve_send_context(account, "filehelper")?;
            let path = Path::new(database);
            crate::data_sources::checked(path, false)?;
            validate_live_source(path, &context.sender_id)?;
            let pin = crate::live_source::LiveSourcePin::open(path)?;
            pin.require(source_fingerprint)?;
            let result = crate::live_reader::calendar_cancellable(
                worker, hash, path, key, selection, cancel,
            )?;
            if p.resolve_send_context(account, "filehelper")? != context {
                return Err(AppError::new(
                    "NATIVE_CONTEXT_CHANGED",
                    "读取期间当前登录身份已变化",
                ));
            }
            Ok(result)
        })
    }
    fn search_live_messages(
        &self,
        account: &str,
        database: &str,
        key: &[u8],
        source_fingerprint: &str,
        selection: &werc_domain::live::LiveSearchSelection,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveSearchWindow> {
        let (worker, hash) = self.live_reader.as_ref().ok_or_else(|| {
            AppError::new("LIVE_READER_UNAVAILABLE", "本次构建未装配在线读取组件")
        })?;
        self.with(|p| {
            let context = p.resolve_send_context(account, "filehelper")?;
            let path = Path::new(database);
            crate::data_sources::checked(path, false)?;
            validate_live_source(path, &context.sender_id)?;
            let pin = crate::live_source::LiveSourcePin::open(path)?;
            pin.require(source_fingerprint)?;
            let result = crate::live_reader::search_messages_cancellable(
                worker, hash, path, key, selection, cancel,
            )?;
            if p.resolve_send_context(account, "filehelper")? != context {
                return Err(AppError::new(
                    "NATIVE_CONTEXT_CHANGED",
                    "读取期间当前登录身份已变化",
                ));
            }
            Ok(result)
        })
    }
    fn read_live_conversation(
        &self,
        account: &str,
        database: &str,
        key: &[u8],
        source_fingerprint: &str,
        selection: &werc_domain::live::LiveConversationSelection,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveWindow> {
        let (worker, hash) = self.live_reader.as_ref().ok_or_else(|| {
            AppError::new("LIVE_READER_UNAVAILABLE", "本次构建未装配在线读取组件")
        })?;
        self.with(|p| {
            let context = p.resolve_send_context(account, "filehelper")?;
            let path = Path::new(database);
            crate::data_sources::checked(path, false)?;
            validate_live_source(path, &context.sender_id)?;
            let source_pin = crate::live_source::LiveSourcePin::open(path)?;
            source_pin.require(source_fingerprint)?;
            let window = crate::live_reader::conversation_window_cancellable(
                worker,
                hash,
                path,
                key,
                self.live_window_rows
                    .unwrap_or(werc_domain::live::LIVE_WINDOW_MAX_ROWS),
                selection,
                cancel,
            )?;
            if p.resolve_send_context(account, "filehelper")? != context {
                return Err(AppError::new(
                    "NATIVE_CONTEXT_CHANGED",
                    "读取期间当前登录身份已变化",
                ));
            }
            Ok(window)
        })
    }
    fn decode_received_voice(&self, silk: &[u8]) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        crate::voice_codec::decode(silk)
    }
    fn encode_voice(&self, wav: &[u8]) -> AppResult<werc_domain::voice::EncodedVoice> {
        crate::voice_codec::encode(wav)
    }
    fn submit_image(&self, s: ImageSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_image(s))
    }
    fn reconcile_image(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_image(q, r))
    }
    fn submit_video(&self, s: VideoSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_video(s))
    }
    fn reconcile_video(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_video(q, r))
    }
    fn submit_emoticon(&self, s: EmoticonSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_emoticon(s))
    }
    fn reconcile_emoticon(
        &self,
        q: TextReceiptQuery<'_>,
        r: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_emoticon(q, r))
    }
    fn submit_file(&self, s: FileSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_file(s))
    }
    fn reconcile_file(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_file(q, r))
    }
    fn submit_voice(&self, s: VoiceSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_voice(s))
    }
    fn reconcile_voice(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_voice(q, r))
    }
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        use werc_domain::capability::{Availability, Evidence};
        let mut c = WindowsWechatProvider.capabilities();
        let profile = self.session.read().ok().and_then(|s|if s.ready {s.provider.as_ref().map(|p|p.native_profile())}else{None});
        let connected = profile.is_some();
        let codec_available = crate::voice_codec::available();
        for item in &mut c {
            if item.id == "messages.read" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected && self.live_reader.is_some() { Availability::Ready } else { Availability::NeedsSetup };
                item.execution_mode = Some("local_read_only".into());
                item.requires_user_action = !connected;
                item.reason = "连接后按需读取已认证的本机微信消息来源，支持会话、搜索和上下文；多个分片分别读取，来源失效时重新打开。".into();
            }
            if item.id == "messages.send_media" {
                item.evidence = Evidence::Implemented;
                item.availability = if !connected {Availability::NeedsSetup} else if codec_available {Availability::Ready} else {Availability::Degraded};
                item.execution_mode = Some("native".into());
                item.requires_user_action = !connected;
                item.reason = "原生图片、语音、文件、视频和表情共用持久发送队列；具体格式和组件条件见各媒体能力。桌面预览批准，授权Bot可直接提交；未知结果只回查。".into();
            }
            if item.id == "media.read" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected && self.live_reader.is_some() {Availability::Degraded} else {Availability::NeedsSetup};
                item.execution_mode = Some("local_read_only".into());
                item.requires_user_action = !connected;
                item.reason = "按账号读取已接收消息、当前来源和已保存资料中的图片、语音、视频、表情与文件；可下载、加密保留和离线回读。具体资源缺失会逐项报告。".into();
            }
            if item.id == "exports.create" {
                item.evidence = Evidence::Implemented;
                item.availability = Availability::Ready;
                item.execution_mode = Some("local".into());
                item.requires_user_action = false;
                item.reason = "聊天、通讯录、群成员、专项记录、报告、朋友圈及文章支持相应格式导出和本地计划；媒体只包含已保存或成功补齐的资源。".into();
            }
            if item.id == "moments.publish" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected {
                    Availability::Degraded
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = true;
                item.reason = "支持本人公开/仅自己可见文字及最多9张图片动态，正文最多10000个UTF-8字节，有图可留空。桌面预览批准，已授予执行权限的Bot可直接提交；未知结果不重发。支持文字、图片和带封面的视频发布，以及精确动态点赞、取消赞、评论、回复和删除本人动态；均使用同一持久队列，原生结果与接收端确认分别记录".into();
            }
            if item.id == "moments.read" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected && self.live_reader.is_some() {
                    Availability::Degraded
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = true;
                item.reason = "需连接当前微信并准备一个消息来源；按需读取同账号本地朋友圈缓存，支持详情、作者/日期筛选、完整缓存正文导出与加密本机保留。支持按需图片/视频/实况片段读取、加密保留与离线媒体导出；不是完整远端朋友圈".into();
            }
            if item.id == "messages.send_video" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected {
                    Availability::Ready
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = !connected;
                item.reason = if connected { "已核验收件人；H.264/AAC MP4、1–60秒、最多8MiB；桌面预览批准，授权Bot可直接提交。原生提交和CDN回读与手机送达分别核验，Unknown不重发" } else { "需显式连接已核验的4.1.15.13原生通道；仅个人非商业自用" }.into();
            }
            if item.id == "messages.send_emoticon" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected {
                    Availability::Ready
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = !connected;
                item.reason = if connected { "已核验收件人；完整GIF、1024边长、120帧/60秒、最多1MiB；桌面预览批准，授权Bot可直接提交。原生构造及提交已核验，手机端未确认，Unknown不重发" } else { "需显式连接已核验的4.1.15.13原生通道；仅个人非商业自用" }.into();
            }
            if item.id == "messages.send_file" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected {
                    Availability::Ready
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = !connected;
                item.reason = if connected {
                    "已核验收件人；非空附件最多8MiB、文件名最多120 UTF-8字节；桌面预览批准，授权Bot可直接提交，原生提交不等于自动送达"
                } else { "需显式连接已核验的4.1.15.13原生通道；仅个人非商业自用" }.into();
            }
            if item.id == "messages.send_image" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected {
                    Availability::Ready
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = !connected;
                item.reason = if connected {
                    "已核验收件人静态PNG，最多8MiB、4096边长和4194304像素；桌面预览批准，授权Bot可直接提交，原生提交不等于自动送达"
                } else { "需显式连接已核验的4.1.15.13原生通道；仅个人非商业自用" }.into();
            }
            if item.id == "messages.send_voice" {
                item.evidence = Evidence::Implemented;
                item.availability = if connected && codec_available {
                    Availability::Ready
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = !connected || !codec_available;
                item.reason = if !codec_available {
                    "需同批通过摘要校验的SILK编码组件"
                } else if !connected {
                    "需显式连接已核验的4.1.15.13原生通道；仅个人非商业自用"
                } else {
                    "已核验收件人；16kHz单声道PCM16、20毫秒整帧、最长60秒；桌面批准或授权Bot直接提交，原生提交不等于自动送达"
                }.into();
            }
            if ["messages.send_text", "messages.observe"].contains(&item.id.as_str()) {
                item.evidence = Evidence::Implemented;
                item.availability = if connected {
                    Availability::Ready
                } else {
                    Availability::NeedsSetup
                };
                item.execution_mode = Some("native".into());
                item.requires_user_action = !connected;
                item.reason = if connected && item.id == "messages.observe" {
                    "按当前登录身份和实际会话接收文字及有界媒体元数据，图片、普通附件和MP4视频可按需下载与加密回读；语音内联缓存及既有媒体库恢复可用，支持表情缓存按需读取和来源补读；缺口只按可证明的覆盖范围报告"
                } else if connected {
                    "文件传输助手、本人或已认证会话的文字发送，单条10000 UTF-8字节；精确任务编号回查可确认服务端接收；接收端送达独立记录，未知结果不重发"
                } else {
                    "需显式连接支持构建的原生通道；仅个人非商业自用"
                }
                .into();
            }
        }
        if let Some(profile)=profile {
            for capability in profile.capabilities() {
                if let Some(existing)=c.iter_mut().find(|c|c.id==capability.id) {
                    existing.availability=capability.availability;
                    existing.reason=capability.reason;
                } else { c.push(capability); }
            }

        }
        c
    }
    fn prepared_recipients(&self, account: &str) -> AppResult<Vec<PreparedRecipient>> {
        self.with(|p| p.prepared_recipients(account))
    }
    fn prepare_client_recipient(
        &self,
        account: &str,
        target: &str,
    ) -> AppResult<PreparedRecipient> {
        self.with(|p| p.prepare_client_target(account, target))
    }
    fn validate_receipt_context(&self, account: &str, context: &SendContext) -> AppResult<()> {
        self.with(|p| p.validate_receipt_context(account, context))
    }
    fn prepare_recipient(
        &self,
        account: &str,
        target: &str,
        sources: &[(String, String)],
    ) -> AppResult<PreparedRecipient> {
        self.with(|p| {
            let context = p.resolve_send_context(account, "filehelper")?;
            if sources.is_empty() || sources.len() > 16 {
                return Err(AppError::new(
                    "NATIVE_TARGET_NOT_VERIFIED",
                    "缺少认证会话来源",
                ));
            }
            let mut pins = Vec::with_capacity(sources.len());
            for (database, fingerprint) in sources {
                let path = Path::new(database);
                validate_live_source(path, &context.sender_id)?;
                let pin = crate::live_source::LiveSourcePin::open(path)?;
                pin.require(fingerprint)?;
                pins.push(pin);
            }
            p.prepare_verified_recipient(account, target, pins)
        })
    }
    fn resolve_send_context(&self, a: &str, t: &str) -> AppResult<SendContext> {
        self.with(|p| p.resolve_send_context(a, t))
    }
    fn submit_moment_action(
        &self,
        s: TextSubmission<'_>,
        action: &MomentAction,
    ) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_moment_action(s, action))
    }
    fn reconcile_moment_action(
        &self,
        q: TextReceiptQuery<'_>,
        r: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_moment_action(q, r))
    }
    fn submit_video_moment(&self, s: MomentVideoSubmission<'_>, visibility: MomentVisibility) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_video_moment(s, visibility))
    }
    fn submit_moment(
        &self,
        s: MomentImageSubmission<'_>,
        visibility: MomentVisibility,
    ) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_moment(s, visibility))
    }
    fn submit_private_image_moment(&self, s: MomentImageSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_private_image_moment(s))
    }
    fn submit_private_moment(&self, s: TextSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_private_moment(s))
    }
    fn reconcile_private_moment(
        &self,
        q: TextReceiptQuery<'_>,
        r: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_private_moment(q, r))
    }
    fn submit_quote(&self, s: QuoteSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_quote(s))
    }
    fn submit_mentions(&self, s: MentionSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_mentions(s))
    }
    fn submit_text(&self, s: TextSubmission<'_>) -> AppResult<SendReceipt> {
        self.with(|p| p.submit_text(s))
    }
    fn reconcile_text(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.with(|p| p.reconcile_text(q, r))
    }
    fn poll_receive(&self, a: &str) -> AppResult<ReceiveBatch> {
        self.with(|p| p.poll_receive(a))
    }
    fn receive_health(&self) -> AppResult<Option<ReceiveHealth>> {
        let s = self.status()?;
        Ok(Some(ReceiveHealth {
            connected: s.connected,
            error_code: s.error_code,
        }))
    }
    fn acknowledge_receive(&self, a: &str, e: &str, s: u64) -> AppResult<()> {
        self.with(|p| p.acknowledge_receive(a, e, s))
    }
    fn offline_ready(&self, p: &str) -> AppResult<bool> {
        WindowsWechatProvider.offline_ready(p)
    }
    fn acquire_database_key(
        &self,
        i: &ClientInstallation,
        p: u32,
        d: &str,
    ) -> AppResult<AcquiredDatabaseKey> {
        WindowsWechatProvider.acquire_database_key(i, p, d)
    }
    fn database_salt(&self, p: &str) -> AppResult<[u8; 16]> {
        WindowsWechatProvider.database_salt(p)
    }
    fn acquire_database_key_controlled(
        &self, installation: &ClientInstallation, process_id: u32, database: &str,
        control: &werc_application::ports::KeyScanControl<'_>,
    ) -> AppResult<AcquiredDatabaseKey> {
        WindowsWechatProvider.acquire_database_key_controlled(installation, process_id, database, control)
    }
    fn read_database(&self, r: &HistoryReadRequest) -> AppResult<HistoryBundle> {
        WindowsWechatProvider.read_database(r)
    }
    fn stream_database(
        &self,
        r: &HistoryReadRequest,
        v: &mut dyn FnMut(HistoryReadEvent) -> AppResult<()>,
    ) -> AppResult<()> {
        WindowsWechatProvider.stream_database(r, v)
    }
    fn discover(&self) -> AppResult<DiscoveryReport> {
        WindowsWechatProvider.discover()
    }
    fn inspect(&self, p: &str) -> AppResult<ClientInstallation> {
        WindowsWechatProvider.inspect(p)
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.unprotect_key(c, k)
    }
}

fn validate_live_source(path: &Path, sender: &str) -> AppResult<()> {
    let valid = (|| {
        let name = path.file_name()?.to_str()?;
        let digits = name.strip_prefix("message_")?.strip_suffix(".db")?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let folder = path.parent()?;
        if folder.file_name()?.to_str()? != "message" {
            return None;
        }
        let storage = folder.parent()?;
        if storage.file_name()?.to_str()? != "db_storage" {
            return None;
        }
        let root = storage.parent()?.file_name()?.to_str()?;
        if sender.is_empty()
            || (root != sender
                && !root
                    .strip_prefix(sender)
                    .is_some_and(|suffix| suffix.starts_with('_') && suffix.len() > 1))
        {
            return None;
        }
        Some(())
    })()
    .is_some();
    if valid {
        Ok(())
    } else {
        Err(AppError::new(
            "LIVE_SOURCE_IDENTITY_MISMATCH",
            "所选消息库目录与当前原生登录身份不一致",
        ))
    }
}

#[cfg(test)]
mod live_source_tests {
    use super::*;
    #[test]
    fn exact_account_boundary_and_message_layout_are_required() {
        assert!(
            validate_live_source(
                Path::new("C:/owned/wxid_owned_abcd/db_storage/message/message_0.db"),
                "wxid_owned"
            )
            .is_ok()
        );
        for path in [
            "C:/owned/wxid_owned2/db_storage/message/message_0.db",
            "C:/owned/wxid_other/db_storage/message/message_0.db",
            "C:/owned/wxid_owned_abcd/db_storage/contact/message_0.db",
            "C:/owned/wxid_owned_abcd/db_storage/message/message_fts.db",
        ] {
            assert!(validate_live_source(Path::new(path), "wxid_owned").is_err());
        }
    }
}

#[cfg(test)]
mod session_concurrency_tests {
    use super::*;
    #[test]
    fn storage_errors_keep_channel_and_identity_errors_disconnect() {
        let managed = ManagedWindowsWechatProvider::default();
        {
            let mut s = managed.session.write().unwrap();
            s.provider = Some(crate::native_messages::owned_pending_download_provider());
            s.account = Some("owned".into());
            s.ready = true;
        }
        for error in ["INTERNAL_ERROR", "KEY_PROTECTION_FAILED"] {
            managed.receive_failed("owned", error).unwrap();
            let status = managed.status().unwrap();
            assert!(status.connected);
            assert_eq!(status.error_code.as_deref(), Some(error));
            managed.resolve_send_context("owned", "filehelper").unwrap();
            managed.receive_progressed("owned").unwrap();
            assert!(managed.status().unwrap().error_code.is_none());
        }
        managed
            .receive_failed("other", "NATIVE_IDENTITY_CHANGED")
            .unwrap();
        assert!(managed.status().unwrap().connected);
        managed
            .receive_failed("owned", "NATIVE_IDENTITY_CHANGED")
            .unwrap();
        assert!(!managed.status().unwrap().connected);
    }
    #[test]
    fn disconnect_cancels_download_then_joins_host_before_returning() {
        let managed = ManagedWindowsWechatProvider::default();
        managed.session.write().unwrap().provider =
            Some(crate::native_messages::owned_pending_download_provider());
        let context = managed.resolve_send_context("owned", "filehelper").unwrap();
        let descriptor = werc_domain::received_media::ReceivedImageDescriptor {
            file_key: zeroize::Zeroizing::new("aabb".into()),
            aes_key: zeroize::Zeroizing::new("ab".repeat(16)),
            declared_size: 20,
        };
        std::thread::scope(|scope| {
            let job = scope.spawn(|| managed.download_received_image(&context, &descriptor));
            // Concurrent status/context reads must remain available while the
            // responder deliberately keeps the download pending forever.
            std::thread::sleep(std::time::Duration::from_millis(150));
            assert_eq!(
                managed.resolve_send_context("owned", "filehelper").unwrap(),
                context
            );
            let start = std::time::Instant::now();
            managed.disconnect(None).unwrap();
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            assert!(job.join().unwrap().is_err());
            assert!(managed.session.read().unwrap().provider.is_none());
            assert!(!managed.status().unwrap().connected);
        });
    }
}
