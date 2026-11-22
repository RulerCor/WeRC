use werc_domain::capability::Capability;
use werc_domain::connection::{ClientBinding, ClientInstallation, DiscoveryReport};
use werc_domain::{Account, AppError, AppResult, Event, Principal, TokenInfo};

pub struct KeyScanControl<'a> {
    pub cancel: &'a std::sync::atomic::AtomicBool,
    pub deadline: std::time::Instant,
    pub progress: &'a (dyn Fn(u64) + Send + Sync),
}
impl KeyScanControl<'_> {
    pub fn check(&self) -> AppResult<()> {
        if self.cancel.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::new("QUERY_CANCELED", "取钥已取消，未保存本次候选"));
        }
        if std::time::Instant::now() >= self.deadline {
            return Err(AppError::new("KEY_SCAN_LIMIT", "取钥扫描已达到所选时间上限"));
        }
        Ok(())
    }
}

pub trait Repository: Send + Sync {
    fn settings_change_by_key(&self,_account:&str,_creator:&str,_digest:&str)->AppResult<Option<werc_domain::wechat_settings::SettingsChange>> { Ok(None) }
    fn settings_change_save(&self,_change:&werc_domain::wechat_settings::SettingsChange)->AppResult<()> { Err(AppError::new("SETTINGS_STORE_UNAVAILABLE","设置变更记录存储未装配")) }
    fn settings_changes(&self,_account:&str,_before:Option<i64>,_limit:u32)->AppResult<Vec<(i64,werc_domain::wechat_settings::SettingsChange)>> { Err(AppError::new("SETTINGS_STORE_UNAVAILABLE","设置变更记录存储未装配")) }
    fn settings_change(&self,_account:&str,_id:&str)->AppResult<Option<werc_domain::wechat_settings::SettingsChange>> { Ok(None) }
    fn library_import(
        &self,
        _account: &str,
        _documents: &mut dyn Iterator<Item = AppResult<werc_domain::library::ProtectedDocument>>,
    ) -> AppResult<u64> {
        Err(AppError::new("LIBRARY_UNAVAILABLE", "资料存储未装配"))
    }
    fn library_save(
        &self,
        _documents: &[werc_domain::library::ProtectedDocument],
        _connection: Option<&werc_domain::library::DocumentConnection>,
    ) -> AppResult<()> {
        Err(AppError::new("LIBRARY_UNAVAILABLE", "资料存储未装配"))
    }
    fn library_get(
        &self,
        _account: &str,
        _id: &str,
    ) -> AppResult<werc_domain::library::ProtectedDocument> {
        Err(AppError::new("NOT_FOUND", "资料不存在"))
    }
    fn library_count(&self, account: &str, kind: &str) -> AppResult<u64>;
    fn library_list(
        &self,
        _account: &str,
        _kind: &str,
        _before: Option<i64>,
        _limit: u32,
    ) -> AppResult<werc_domain::library::DocumentPage> {
        Err(AppError::new("LIBRARY_UNAVAILABLE", "资料存储未装配"))
    }
    fn library_delete(&self, _account: &str, _id: &str, _revision: i64) -> AppResult<()> {
        Err(AppError::new("LIBRARY_UNAVAILABLE", "资料存储未装配"))
    }
    fn received_video_info(
        &self,
        account: &str,
        observation: &str,
    ) -> AppResult<Option<(werc_domain::video::VideoAsset, String)>>;
    fn received_video(
        &self,
        account: &str,
        observation: &str,
    ) -> AppResult<werc_domain::video::ProtectedVideo>;
    fn save_received_video(
        &self,
        data: &werc_domain::video::ProtectedVideo,
        binding: &werc_domain::video::VideoBinding<'_>,
    ) -> AppResult<()>;
    fn delete_received_video(&self, account: &str, observation: &str) -> AppResult<()>;
    fn received_file_link(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>>;
    fn save_received_file(
        &self,
        file: &werc_domain::file::ProtectedFile,
        binding: &werc_domain::received_media::ReceivedImageBinding<'_>,
    ) -> AppResult<()>;
    fn save_received_emoticon(
        &self,
        _asset: &werc_domain::file::ProtectedFile,
        _binding: &werc_domain::received_media::ReceivedImageBinding<'_>,
        _key_revision: i64,
    ) -> AppResult<()> {
        Err(AppError::new(
            "RECEIVED_MEDIA_UNAVAILABLE",
            "未装配接收表情保存",
        ))
    }
    fn received_voice_link(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>>;
    fn received_image_link(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>>;
    fn save_received_image_file(
        &self,
        file: &werc_domain::file::ProtectedFile,
        binding: &werc_domain::received_media::ReceivedImageBinding<'_>,
    ) -> AppResult<()>;
    fn save_image(&self, asset: &werc_domain::image::ProtectedImage) -> AppResult<()>;
    fn images(&self, account: &str) -> AppResult<Vec<werc_domain::image::ImageAsset>>;
    fn image(&self, account: &str, id: &str) -> AppResult<werc_domain::image::ProtectedImage>;
    fn delete_image(&self, account: &str, id: &str) -> AppResult<()>;

    fn save_file(&self, asset: &werc_domain::file::ProtectedFile) -> AppResult<()>;
    fn save_live_file(
        &self,
        _asset: &werc_domain::file::ProtectedFile,
        _revision: i64,
        _key_revision: i64,
    ) -> AppResult<()> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配当前连接媒体保存",
        ))
    }
    fn files(&self, account: &str) -> AppResult<Vec<werc_domain::file::FileAsset>>;
    fn file(&self, account: &str, id: &str) -> AppResult<werc_domain::file::ProtectedFile>;
    fn delete_file(&self, account: &str, id: &str) -> AppResult<()>;

    fn commit_recovery(
        &self,
        data: &werc_domain::recovery::RecoveryCommit<'_>,
    ) -> AppResult<werc_domain::recovery::RecoverySummary>;
    fn recovery_summaries(
        &self,
        account: &str,
    ) -> AppResult<Vec<werc_domain::recovery::RecoverySummary>>;
    fn recovery(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<werc_domain::recovery::ProtectedRecovery>;
    fn source_owner(&self, fingerprint: &str) -> AppResult<Option<String>>;
    fn bind_current_source(&self, account: &str, revision: i64, fingerprint: &str)
    -> AppResult<()>;
    fn native_identity(&self, account: &str) -> AppResult<Option<String>>;
    fn bind_native_identity(&self, account: &str, revision: i64, token: &str) -> AppResult<()>;
    fn receive_cursor(
        &self,
        account: &str,
    ) -> AppResult<Option<werc_domain::receive::ReceiveCursor>>;
    fn commit_receive(&self, data: &werc_domain::receive::ReceiveCommit<'_>) -> AppResult<bool>;
    fn received_texts(
        &self,
        account: &str,
        after: i64,
        limit: u32,
    ) -> AppResult<Vec<(i64, werc_domain::receive::ProtectedReceivedText)>>;
    fn received_text_by_id(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<Option<werc_domain::receive::ProtectedReceivedText>>;
    fn current_connection(&self) -> AppResult<werc_domain::current::CurrentConnection>;
    fn select_current_connection(&self, account: &str, revision: &str) -> AppResult<()>;
    fn send_secret(&self, account: &str, candidate: Option<&[u8]>) -> AppResult<Option<Vec<u8>>>;
    fn next_queued_send(&self) -> AppResult<Option<werc_domain::outbox::SendStatus>>;
    fn create_send(
        &self,
        data: &werc_domain::outbox::ProtectedSend,
    ) -> AppResult<werc_domain::outbox::SendStatus>;
    fn send_status(&self, account: &str, id: &str) -> AppResult<werc_domain::outbox::SendStatus>;
    fn send_payload(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<werc_domain::outbox::ProtectedSend>;
    fn sends(&self, account: &str) -> AppResult<Vec<werc_domain::outbox::SendStatus>>;
    fn send_by_idempotency(
        &self,
        account: &str,
        creator: &str,
        digest: &str,
    ) -> AppResult<Option<werc_domain::outbox::SendStatus>> {
        Ok(self
            .sends(account)?
            .into_iter()
            .find(|s| s.metadata.creator_id == creator && s.metadata.idempotency_digest == digest))
    }
    fn send_history_page(
        &self,
        _account: &str,
        _before: Option<i64>,
        _limit: u32,
    ) -> AppResult<werc_domain::outbox::SendPage> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "发送历史分页未装配",
        ))
    }
    fn pending_receipt_page(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<werc_domain::outbox::SendPage> {
        let mut page = self.send_history_page(account, before, limit)?;
        page.data.retain(|s| {
            matches!(
                s.state,
                werc_domain::outbox::SendState::Unknown
                    | werc_domain::outbox::SendState::ServerAccepted
            )
        });
        Ok(page)
    }
    fn approve_send(
        &self,
        account: &str,
        id: &str,
        digest: &str,
        context: &str,
        until: i64,
        proof: &[u8],
    ) -> AppResult<werc_domain::outbox::SendStatus>;
    fn approve_timed_send(
        &self,
        account: &str,
        id: &str,
        digest: &str,
        context: &str,
        timing: &werc_domain::outbox::SendTiming,
        proof: &[u8],
    ) -> AppResult<werc_domain::outbox::SendStatus> {
        if timing.scheduled_at.is_some() {
            return Err(AppError::new(
                "SCHEDULING_UNAVAILABLE",
                "当前队列未提供持久定时发送",
            ));
        }
        self.approve_send(account, id, digest, context, timing.expires_at, proof)
    }
    fn send_approval(&self, account: &str, id: &str) -> AppResult<Option<Vec<u8>>>;
    fn start_send(
        &self,
        expected: &werc_domain::outbox::SendStatus,
        attempt: &str,
    ) -> AppResult<werc_domain::outbox::SendStatus>;
    fn record_send_receipt(
        &self,
        account: &str,
        receipt: &werc_domain::outbox::ProtectedReceipt,
    ) -> AppResult<werc_domain::outbox::SendStatus>;
    fn send_receipt(&self, account: &str, id: &str) -> AppResult<Option<Vec<u8>>>;
    fn cancel_send(&self, account: &str, id: &str) -> AppResult<werc_domain::outbox::SendStatus>;
    fn mark_send_unknown(
        &self,
        account: &str,
        id: &str,
        attempt: &str,
    ) -> AppResult<werc_domain::outbox::SendStatus>;
    fn recover_sends(&self) -> AppResult<usize>;
    fn projection(
        &self,
        account: &str,
        generation: &str,
    ) -> AppResult<Option<werc_domain::projection::ProtectedProjection>>;
    fn begin_projection(
        &self,
        data: &werc_domain::projection::ProtectedProjection,
    ) -> AppResult<()>;
    fn append_projection(
        &self,
        account: &str,
        id: &str,
        rows: &[werc_domain::projection::ProtectedProjectionRow],
    ) -> AppResult<()>;
    fn finish_projection(
        &self,
        data: &werc_domain::projection::ProtectedProjection,
    ) -> AppResult<()>;
    fn projection_row(
        &self,
        account: &str,
        id: &str,
        token: &str,
    ) -> AppResult<Option<werc_domain::projection::ProtectedProjectionRow>>;
    fn remove_projection(&self, account: &str, id: &str) -> AppResult<()>;
    fn clear_projections(&self, account: &str) -> AppResult<usize>;
    fn setup_payload(&self) -> AppResult<Option<Vec<u8>>>;
    fn save_setup_payload(&self, payload: Option<&[u8]>) -> AppResult<()>;
    fn setup_account(&self, label: &str) -> AppResult<Account>;
    fn setup_account_for_source(&self, label: &str, fingerprint: &str) -> AppResult<Account>;
    fn setup_dismissed(&self) -> AppResult<bool>;
    fn dismiss_setup(&self) -> AppResult<()>;
    fn accounts(&self) -> AppResult<Vec<Account>>;
    fn register_account(&self, label: &str) -> AppResult<Account>;
    fn remove_account(&self, id: &str) -> AppResult<()>;
    fn create_token(&self, info: &TokenInfo, digest: &str) -> AppResult<()>;
    fn tokens(&self) -> AppResult<Vec<TokenInfo>>;
    fn revoke_token(&self, id: &str) -> AppResult<()>;
    fn authenticate(&self, digest: &str) -> AppResult<Principal>;
    fn events(&self, after: i64, account: &str, limit: u32) -> AppResult<Vec<Event>>;
    fn bindings(&self) -> AppResult<Vec<ClientBinding>>;
    fn save_binding(&self, binding: &ClientBinding) -> AppResult<()>;
    fn refresh_binding_preserving_key(&self, _old: &ClientBinding, _new: &ClientBinding, _old_key: Option<&[u8]>, _new_key: Option<&[u8]>, _revision: i64) -> AppResult<()> {
        Err(AppError::new("CONNECTION_CHANGED", "当前存储不支持保留密钥更新绑定"))
    }
    fn remove_binding(&self, account: &str) -> AppResult<()>;
    fn store_key(&self, binding: &ClientBinding, protected: &[u8]) -> AppResult<()>;
    fn key_revision(&self, account: &str) -> AppResult<i64>;
    fn store_acquired_key(
        &self,
        binding: &ClientBinding,
        protected: &[u8],
        expected_revision: i64,
    ) -> AppResult<()>;
    fn protected_key(&self, account: &str) -> AppResult<Option<(Vec<u8>, String)>>;
    fn remove_key(&self, account: &str) -> AppResult<()>;
    fn save_asset(&self, asset: &werc_domain::media::ProtectedAsset) -> AppResult<()>;
    fn save_live_audio(
        &self,
        _asset: &werc_domain::media::ProtectedAsset,
        _revision: i64,
        _key_revision: i64,
    ) -> AppResult<()> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配当前连接语音保存",
        ))
    }
    fn assets(&self, account: &str) -> AppResult<Vec<werc_domain::media::AudioAsset>>;
    fn asset(&self, account: &str, id: &str) -> AppResult<werc_domain::media::ProtectedAsset>;
    fn delete_asset(&self, account: &str, id: &str) -> AppResult<()>;
    fn queue_operation(
        &self,
        operation: &werc_domain::operation::Operation,
        key: &str,
    ) -> AppResult<werc_domain::operation::Operation>;
    fn operations(&self, account: &str) -> AppResult<Vec<werc_domain::operation::Operation>>;
    /// Exhaustive existence check; UI list limits must not hide active work.
    fn has_pending_tasks(&self, account: &str) -> AppResult<bool>;
    fn operation(&self, account: &str, id: &str) -> AppResult<werc_domain::operation::Operation>;
    fn claim_operation(&self) -> AppResult<Option<werc_domain::operation::Operation>>;
    fn finish_operation(&self, operation: &werc_domain::operation::Operation) -> AppResult<()>;
    fn cancel_operation(&self, account: &str, id: &str) -> AppResult<()>;
    fn recover_operations(&self) -> AppResult<usize>;
    fn history_sources(&self, account: &str)
    -> AppResult<Vec<werc_domain::history::HistorySource>>;
    fn history_payload(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<werc_domain::history::ProtectedHistory>;
    fn save_history(&self, data: &werc_domain::history::ProtectedHistory) -> AppResult<()>;
    fn begin_history(&self, data: &werc_domain::history::ProtectedHistory) -> AppResult<()>;
    fn append_history_batch(
        &self,
        account: &str,
        id: &str,
        batch: &werc_domain::history::ProtectedHistoryBatch,
    ) -> AppResult<()>;
    fn finish_history(
        &self,
        data: &werc_domain::history::ProtectedHistory,
        expected_revision: i64,
    ) -> AppResult<()>;
    fn abort_history(&self, account: &str, id: &str) -> AppResult<()>;
    fn history_batch(
        &self,
        account: &str,
        id: &str,
        sequence: u64,
    ) -> AppResult<Option<werc_domain::history::ProtectedHistoryBatch>>;
    fn history_message_states(
        &self,
        account: &str,
        ids: &[String],
    ) -> AppResult<std::collections::BTreeMap<String, werc_domain::history::HistoryMessageState>>;
    fn history_message_digest(&self, account: &str, message_id: &str) -> AppResult<Option<String>>;
    fn history_message_is_current(
        &self,
        account: &str,
        source: &str,
        message_id: &str,
    ) -> AppResult<bool>;
    fn history_index_digest(&self, account: &str, source: &str) -> AppResult<String>;
    fn history_index_digest_controlled(
        &self,
        account: &str,
        source: &str,
        check: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<String> {
        check()?;
        let digest = self.history_index_digest(account, source)?;
        check()?;
        Ok(digest)
    }
    fn history_index_key(
        &self,
        account: &str,
        candidate: Option<&[u8]>,
    ) -> AppResult<Option<Vec<u8>>>;
    fn remove_history(&self, account: &str, id: &str) -> AppResult<()>;
}

pub struct AutomaticMessageSources {
    pub process_id: u32,
    pub databases: Vec<String>,
}
pub trait CapabilityProvider: Send + Sync {
    fn wechat_settings_read(&self,_account:&str)->AppResult<werc_domain::wechat_settings::SettingsSnapshot> { Err(AppError::new("WECHAT_SETTINGS_UNAVAILABLE","当前构建没有已验证的后台原生设置能力")) }
    fn wechat_settings_apply(&self,_account:&str,_target:&werc_domain::wechat_settings::SettingTarget,_epoch:&str)->AppResult<werc_domain::wechat_settings::SettingResult> { Err(AppError::new("WECHAT_SETTINGS_UNAVAILABLE","当前构建没有已验证的后台原生设置能力")) }
    /// Release only this account's native session before deleting its local state.
    fn disconnect_account(&self, _account: &str) -> AppResult<()> { Ok(()) }
    fn recover_image_cache_key(&self,_account:&str,_sample:&[u8],_cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::connection::ImageCacheKey>{Err(AppError::new("IMAGE_KEY_UNAVAILABLE","当前提供方不支持图片密钥恢复"))}
    fn decode_image_cache(&self,_sample:&[u8],_key:&werc_domain::connection::ImageCacheKey,_cancel:&std::sync::atomic::AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{Err(AppError::new("IMAGE_KEY_UNAVAILABLE","当前提供方不支持本地图片解码"))}
    fn asr_status(&self) -> AppResult<werc_domain::asr::AsrStatus> {
        Ok(werc_domain::asr::AsrStatus {
            available: false,
            models: Vec::new(),
            engine: "whisper.cpp",
        })
    }
    /// Providers with replaceable model files override this with their content identity.
    fn asr_cache_identity(
        &self,
        _model: &str,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<String> {
        Ok(self.asr_status()?.engine.into())
    }
    fn transcribe_audio(
        &self,
        _wav: &[u8],
        _model: &str,
        _language: &str,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::asr::Transcript> {
        Err(AppError::new("ASR_UNAVAILABLE", "离线转写组件或模型未安装"))
    }
    fn read_live_group_members(
        &self,
        _account: &str,
        _database: &str,
        _source_fingerprint: &str,
        _selection: &werc_domain::group::LiveGroupSelection,
        _expected_fingerprint: Option<&str>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::group::LiveGroupWindow)> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配本地群成员读取",
        ))
    }

    fn read_live_contacts(
        &self,
        _account: &str,
        _database: &str,
        _source_fingerprint: &str,
        _after: Option<&werc_domain::live::LiveContactContinuation>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::live::LiveContactWindow)> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配当前来源联系人读取",
        ))
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
        if search_query.is_some() {
            return Err(AppError::new(
                "LIVE_QUERY_UNAVAILABLE",
                "当前读取组件尚未提供联系人搜索",
            ));
        }
        self.read_live_contacts(account, database, source_fingerprint, after, cancel)
    }
    fn read_live_sessions(
        &self,
        _account: &str,
        _database: &str,
        _source_fingerprint: &str,
        _after: Option<&werc_domain::live::LiveSessionContinuation>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::live::LiveSessionWindow)> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配当前来源会话读取",
        ))
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
        if search_query.is_some() {
            return Err(AppError::new(
                "LIVE_QUERY_UNAVAILABLE",
                "当前读取组件尚未提供会话搜索",
            ));
        }
        self.read_live_sessions(account, database, source_fingerprint, after, cancel)
    }
    fn read_live_special_records(
        &self,
        _account: &str,
        _database: &str,
        _source_fingerprint: &str,
        _selection: &werc_domain::special_records::RecordSelection,
        _expected_fingerprint: Option<&str>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::special_records::RecordWindow)> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配微信专项记录读取",
        ))
    }
    fn fetch_article_image(&self,_account:&str,_url:&str,_cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::articles::ArticleImageBytes>{
        Err(AppError::new("ARTICLE_IMAGE_UNAVAILABLE","未装配文章图片下载"))
    }
    fn fetch_contact_avatar(&self,_account:&str,_url:&str,_cancel:&std::sync::atomic::AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{
        Err(AppError::new("CONTACT_AVATAR_UNAVAILABLE","未装配联系人头像读取"))
    }
    fn article_history(&self, _account:&str, _publisher:&str, _cursor:Option<&str>, _cancel:&std::sync::atomic::AtomicBool)
        ->AppResult<werc_domain::article_history::ArticleHistoryPage> {
        Err(AppError::new("ARTICLE_NATIVE_UNAVAILABLE","请先连接已支持的微信原生通道"))
    }
    fn fetch_article(&self, _account:&str, _url:&str, _cancel:&std::sync::atomic::AtomicBool)
        ->AppResult<werc_domain::articles::Article> {
        Err(AppError::new("ARTICLE_COMPONENT_UNAVAILABLE","未装配公众号文章读取组件"))
    }
    /// Bulk readers may amortize database/process setup. Older providers retain normal pages.
    fn read_live_favorites(
        &self,
        _account: &str,
        _database: &str,
        _source_fingerprint: &str,
        _selection: &werc_domain::favorites::FavoriteSelection,
        _expected_fingerprint: Option<&str>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::favorites::FavoriteWindow)> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配微信收藏读取",
        ))
    }
    /// Bulk readers may amortize database/process setup. Older providers retain normal pages.
    fn read_live_moments_batch(
        &self,
        account: &str,
        database: &str,
        source_fingerprint: &str,
        after: Option<&werc_domain::moments::LiveMomentContinuation>,
        _limit: usize,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::moments::LiveMomentWindow)> {
        self.read_live_moments(account, database, source_fingerprint, after, cancel)
    }
    fn read_moment_media(
        &self,
        _account: &str,
        _database: &str,
        _fingerprint: &str,
        _selection: &werc_domain::moments::MomentMediaSelection,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::moments::MomentMediaBytes> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "当前读取组件未提供朋友圈媒体",
        ))
    }
    fn read_live_moments(
        &self,
        _account: &str,
        _database: &str,
        _source_fingerprint: &str,
        _after: Option<&werc_domain::moments::LiveMomentContinuation>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<(String, werc_domain::moments::LiveMomentWindow)> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配当前来源朋友圈读取",
        ))
    }
    fn download_received_video(
        &self,
        _context: &werc_domain::outbox::SendContext,
        _descriptor: &werc_domain::received_media::ReceivedVideoDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "RECEIVED_MEDIA_UNAVAILABLE",
            "未装配原生视频下载通道",
        ))
    }
    fn render_received_emoticon(
        &self,
        _bytes: &[u8],
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "EMOTICON_CODEC_UNAVAILABLE",
            "未装配缓存动画解码组件",
        ))
    }
    fn read_received_emoticon(
        &self,
        _account: &str,
        _context: &werc_domain::outbox::SendContext,
        _descriptor: &werc_domain::received_emoticon::ReceivedEmoticonDescriptor,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "RECEIVED_MEDIA_UNAVAILABLE",
            "未装配本地表情读取组件",
        ))
    }
    fn read_live_emoticon(
        &self,
        _account: &str,
        _database: &str,
        _source: &str,
        _descriptor: &werc_domain::received_emoticon::ReceivedEmoticonDescriptor,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配本地表情读取组件",
        ))
    }
    fn read_live_forward_file(
        &self,
        _account: &str,
        _database: &str,
        _source: &str,
        _descriptor: &werc_domain::message_card::forward_file::ForwardFile,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配合并记录附件缓存读取组件",
        ))
    }
    fn read_live_voice(
        &self,
        _account: &str,
        _database: &str,
        _source_fingerprint: &str,
        _anchor: &werc_domain::live::LiveVoiceAnchor,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<Option<zeroize::Zeroizing<Vec<u8>>>> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配本地语音读取组件",
        ))
    }
    fn download_favorite_media(
        &self,
        _context: &werc_domain::outbox::SendContext,
        _kind: u32,
        _descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "FAVORITE_MEDIA_UNAVAILABLE",
            "未装配原生收藏媒体下载通道",
        ))
    }
    fn download_received_file(
        &self,
        _context: &werc_domain::outbox::SendContext,
        _descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "RECEIVED_MEDIA_UNAVAILABLE",
            "未装配原生附件下载通道",
        ))
    }
    fn download_original_image(
        &self,
        _context: &werc_domain::outbox::SendContext,
        _descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "RECEIVED_MEDIA_UNAVAILABLE",
            "未装配原生原图下载通道",
        ))
    }
    fn download_received_image(
        &self,
        _context: &werc_domain::outbox::SendContext,
        _descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "RECEIVED_MEDIA_UNAVAILABLE",
            "未装配原生图片下载通道",
        ))
    }
    /// Current authenticated login only; paths are discovered locally, not client input.
    fn automatic_message_sources(&self, _account: &str) -> AppResult<AutomaticMessageSources> {
        Err(AppError::new(
            "LIVE_SOURCE_DISCOVERY_UNAVAILABLE",
            "此提供方未提供自动消息来源发现",
        ))
    }
    fn live_source_fingerprint(&self, _database: &str) -> AppResult<String> {
        Err(AppError::new(
            "LIVE_SOURCE_ID_UNAVAILABLE",
            "此提供方未装配来源文件身份校验",
        ))
    }
    fn validate_live_source(&self, _account: &str, _database: &str) -> AppResult<()> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "此提供方未装配在线来源校验",
        ))
    }
    fn live_reader_fingerprint(&self) -> AppResult<String> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "此提供方未装配在线读取引擎",
        ))
    }
    fn read_live_calendar(
        &self,
        _account: &str,
        _database: &str,
        _key: &[u8],
        _source_fingerprint: &str,
        _selection: &werc_domain::live::LiveCalendarSelection,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveCalendarWindow> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配会话日历组件",
        ))
    }
    fn search_live_messages(
        &self,
        _account: &str,
        _database: &str,
        _key: &[u8],
        _source_fingerprint: &str,
        _selection: &werc_domain::live::LiveSearchSelection,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveSearchWindow> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配跨会话搜索组件",
        ))
    }
    fn read_live_conversation(
        &self,
        _account: &str,
        _database: &str,
        _key: &[u8],
        _source_fingerprint: &str,
        _selection: &werc_domain::live::LiveConversationSelection,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveWindow> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "未装配当前来源会话消息读取",
        ))
    }
    fn read_live_filehelper(
        &self,
        _account: &str,
        _database: &str,
        _key: &[u8],
        _source_fingerprint: &str,
        _before: Option<&werc_domain::live::LiveAnchor>,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::live::LiveWindow> {
        Err(AppError::new(
            "LIVE_READER_UNAVAILABLE",
            "此提供方未装配在线读取引擎",
        ))
    }
    fn receive_health(&self) -> AppResult<Option<werc_domain::receive::ReceiveHealth>> {
        Ok(None)
    }
    fn poll_receive(&self, _account: &str) -> AppResult<werc_domain::receive::ReceiveBatch> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未连接原生接收通道",
        ))
    }
    fn acknowledge_receive(&self, _account: &str, _epoch: &str, _sequence: u64) -> AppResult<()> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未连接原生接收通道",
        ))
    }
    /// Exact username supplied by an execution-authorized Bot, never a nickname.
    /// This prepares a target but does not approve or submit any message.
    fn prepare_client_recipient(
        &self,
        _account: &str,
        _target: &str,
    ) -> AppResult<werc_domain::outbox::PreparedRecipient> {
        Err(AppError::new(
            "NATIVE_TARGET_NOT_VERIFIED",
            "当前提供方不支持客户端精确收件人",
        ))
    }
    /// Called only with a retained authenticated conversation and its file identities.
    fn prepare_recipient(
        &self,
        _account: &str,
        _target: &str,
        _sources: &[(String, String)],
    ) -> AppResult<werc_domain::outbox::PreparedRecipient> {
        Err(AppError::new(
            "NATIVE_TARGET_NOT_VERIFIED",
            "当前提供方不支持选择会话收件人",
        ))
    }
    fn prepared_recipients(
        &self,
        _account: &str,
    ) -> AppResult<Vec<werc_domain::outbox::PreparedRecipient>> {
        Err(AppError::new(
            "NATIVE_NOT_CONNECTED",
            "请先连接原生消息通道",
        ))
    }
    /// Receipt reads never renew a recipient lease or authorize another submission.
    fn validate_receipt_context(
        &self,
        account: &str,
        context: &werc_domain::outbox::SendContext,
    ) -> AppResult<()> {
        if self.resolve_send_context(account, &context.target_id)? != *context {
            return Err(AppError::new("CONNECTION_CHANGED", "原提交身份已变化"));
        }
        Ok(())
    }
    fn resolve_send_context(
        &self,
        _account: &str,
        _target: &str,
    ) -> AppResult<werc_domain::outbox::SendContext> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配通过版本、真实身份及目标验真的发送提供方",
        ))
    }
    fn submit_moment_action(
        &self,
        _s: werc_domain::outbox::TextSubmission<'_>,
        _action: &werc_domain::outbox::MomentAction,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "当前提供方不支持朋友圈互动",
        ))
    }
    fn reconcile_moment_action(
        &self,
        _q: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "当前提供方不能回查朋友圈互动；不会重新提交",
        ))
    }
    fn submit_video_moment(&self, _s: werc_domain::outbox::MomentVideoSubmission<'_>,
        _visibility: werc_domain::outbox::MomentVisibility) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new("CAPABILITY_NOT_IMPLEMENTED", "当前提供方不支持朋友圈视频发布"))
    }
    fn submit_moment(
        &self,
        s: werc_domain::outbox::MomentImageSubmission<'_>,
        visibility: werc_domain::outbox::MomentVisibility,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        if visibility != werc_domain::outbox::MomentVisibility::Private {
            return Err(AppError::new(
                "CAPABILITY_NOT_IMPLEMENTED",
                "此提供方不支持公开动态",
            ));
        }
        if s.images.is_empty() {
            self.submit_private_moment(s.submission)
        } else {
            self.submit_private_image_moment(s)
        }
    }
    fn submit_private_image_moment(
        &self,
        _s: werc_domain::outbox::MomentImageSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "当前提供方不支持私密图片动态发布",
        ))
    }
    fn submit_private_moment(
        &self,
        _s: werc_domain::outbox::TextSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "当前版本不支持私密文字动态发布",
        ))
    }
    fn reconcile_private_moment(
        &self,
        _q: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此提供方不能回查朋友圈发布；不会重新提交",
        ))
    }
    fn submit_quote(
        &self,
        _s: werc_domain::outbox::QuoteSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生引用发送",
        ))
    }
    fn submit_mentions(
        &self,
        _s: werc_domain::outbox::MentionSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生群成员提醒",
        ))
    }
    fn submit_text(
        &self,
        _submission: werc_domain::outbox::TextSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生发送；没有切换可见自动化",
        ))
    }
    fn decode_received_voice(&self, _silk: &[u8]) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        Err(AppError::new(
            "VOICE_CODEC_UNAVAILABLE",
            "未装配语音解码组件",
        ))
    }
    fn encode_voice(&self, _wav: &[u8]) -> AppResult<werc_domain::voice::EncodedVoice> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生语音编码器",
        ))
    }
    fn submit_voice(
        &self,
        _submission: werc_domain::outbox::VoiceSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生语音提交",
        ))
    }
    fn submit_image(
        &self,
        _submission: werc_domain::outbox::ImageSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生图片提交",
        ))
    }
    fn reconcile_image(
        &self,
        _query: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "图片结果保持未知，不重发",
        ))
    }
    fn submit_video(
        &self,
        _submission: werc_domain::outbox::VideoSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生视频提交",
        ))
    }
    fn reconcile_video(
        &self,
        _query: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "视频结果保持未知，不重发",
        ))
    }
    fn submit_emoticon(
        &self,
        _submission: werc_domain::outbox::EmoticonSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生表情提交",
        ))
    }
    fn reconcile_emoticon(
        &self,
        _query: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "表情结果保持未知，不重发",
        ))
    }
    fn submit_file(
        &self,
        _submission: werc_domain::outbox::FileSubmission<'_>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "未装配原生文件提交",
        ))
    }
    fn reconcile_file(
        &self,
        _query: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "文件结果保持未知，不重发",
        ))
    }
    fn reconcile_voice(
        &self,
        _query: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "语音结果保持未知，不重发",
        ))
    }
    fn reconcile_text(
        &self,
        _query: werc_domain::outbox::TextReceiptQuery<'_>,
        _reference: Option<&str>,
    ) -> AppResult<werc_domain::outbox::SendReceipt> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此提供方不能可靠回查；结果保持未知，不重发",
        ))
    }
    fn offline_ready(&self, _path: &str) -> AppResult<bool> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此平台未实现离线条件检查",
        ))
    }
    fn acquire_database_key(
        &self,
        _installation: &ClientInstallation,
        _process_id: u32,
        _database: &str,
    ) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此平台未实现只读取钥",
        ))
    }
    fn acquire_database_key_controlled(
        &self,
        installation: &ClientInstallation,
        process_id: u32,
        database: &str,
        control: &KeyScanControl<'_>,
    ) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
        control.check()?;
        let key = self.acquire_database_key(installation, process_id, database)?;
        control.check()?;
        Ok(key)
    }
    fn database_salt(&self, _path: &str) -> AppResult<[u8; 16]> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此平台未实现数据库头检查",
        ))
    }
    fn read_database(
        &self,
        _request: &werc_domain::history::HistoryReadRequest,
    ) -> AppResult<werc_domain::history::HistoryBundle> {
        Err(AppError::new(
            "READER_UNAVAILABLE",
            "当前未装配经过完整性校验的只读数据执行器",
        ))
    }
    fn stream_database(
        &self,
        request: &werc_domain::history::HistoryReadRequest,
        visitor: &mut dyn FnMut(werc_domain::history::HistoryReadEvent) -> AppResult<()>,
    ) -> AppResult<()> {
        use sha2::{Digest, Sha256};
        use werc_domain::history::HistoryReadEvent;
        let data = self.read_database(request)?;
        visitor(HistoryReadEvent::Begin {
            schema: data.schema.clone(),
            source_sha256: data.source_sha256.clone(),
            encryption: data.encryption.clone(),
            wal_committed_frames: data.wal_committed_frames,
        })?;
        let (message_count, contact_count, session_count) = (
            data.messages.len(),
            data.contacts.len(),
            data.sessions.len(),
        );
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&data).map_err(|_| AppError::internal())?,
        ));
        visitor(HistoryReadEvent::Batch {
            sequence: 0,
            digest,
            data,
        })?;
        visitor(HistoryReadEvent::End {
            batches: 1,
            message_count,
            contact_count,
            session_count,
        })
    }
    fn capabilities(&self) -> Vec<Capability>;
    fn discover(&self) -> AppResult<DiscoveryReport> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此平台未实现客户端发现",
        ))
    }
    fn inspect(&self, _path: &str) -> AppResult<ClientInstallation> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此平台未实现客户端检查",
        ))
    }
    fn protect_key(&self, _account: &str, _key: &[u8]) -> AppResult<Vec<u8>> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此平台未实现受保护密钥存储",
        ))
    }
    fn unprotect_key(&self, _account: &str, _protected: &[u8]) -> AppResult<Vec<u8>> {
        Err(AppError::new(
            "CAPABILITY_NOT_IMPLEMENTED",
            "此平台未实现受保护密钥存储",
        ))
    }
}
