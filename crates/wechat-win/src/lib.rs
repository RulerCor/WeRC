pub mod asr;
pub mod cuda_devices;
pub mod sherpa_asr;
mod articles;
pub mod data_sources;
pub mod discovery;
pub mod native_profiles;
mod emoticon_cache;
pub mod key_acquisition;
pub mod keys;
pub mod image_cache;
#[cfg(windows)]
pub mod live_reader;
#[cfg(windows)]
pub mod live_source;
#[cfg(windows)]
pub mod managed_messages;
#[cfg(windows)]
pub mod native_messages;
pub mod reader;
pub mod setup;
#[cfg(windows)]
pub mod voice_codec;

use werc_application::ports::CapabilityProvider;
use werc_domain::capability::{Availability, Capability, Evidence};
use werc_domain::{
    AppResult,
    connection::{ClientInstallation, DiscoveryReport},
};

pub struct WindowsWechatProvider;

impl CapabilityProvider for WindowsWechatProvider {
    fn decode_image_cache(&self,sample:&[u8],key:&werc_domain::connection::ImageCacheKey,cancel:&std::sync::atomic::AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{image_cache::decode(sample,key,cancel)}
    fn fetch_contact_avatar(&self,_account:&str,url:&str,cancel:&std::sync::atomic::AtomicBool)->AppResult<zeroize::Zeroizing<Vec<u8>>>{crate::articles::fetch_avatar(url,cancel)}
    fn fetch_article_image(&self,_account:&str,url:&str,cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::articles::ArticleImageBytes>{crate::articles::fetch_image(url,cancel)}
    fn fetch_article(&self,_account:&str,url:&str,cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::articles::Article>{
        articles::fetch_public(url,cancel)
    }
    fn asr_status(&self) -> AppResult<werc_domain::asr::AsrStatus> {
        asr::status()
    }
    fn asr_cache_identity(
        &self,
        model: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<String> {
        asr::cache_identity(model, cancel)
    }
    fn transcribe_audio(
        &self,
        wav: &[u8],
        model: &str,
        language: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<werc_domain::asr::Transcript> {
        asr::transcribe(wav, model, language, cancel)
    }
    #[cfg(windows)]
    fn render_received_emoticon(
        &self,
        bytes: &[u8],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        crate::emoticon_codec::render(bytes, cancel)
    }

    #[cfg(windows)]
    fn decode_received_voice(&self, silk: &[u8]) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        voice_codec::decode(silk)
    }
    fn offline_ready(&self, path: &str) -> AppResult<bool> {
        setup::offline_ready(path)
    }
    fn acquire_database_key(
        &self,
        installation: &ClientInstallation,
        process_id: u32,
        database: &str,
    ) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
        key_acquisition::acquire(installation, process_id, database)
    }
    fn database_salt(&self, path: &str) -> AppResult<[u8; 16]> {
        key_acquisition::database_salt(path)
    }
    fn acquire_database_key_controlled(
        &self, installation: &ClientInstallation, process_id: u32, database: &str,
        control: &werc_application::ports::KeyScanControl<'_>,
    ) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
        key_acquisition::acquire_controlled(installation, process_id, database, control)
    }
    fn read_database(
        &self,
        request: &werc_domain::history::HistoryReadRequest,
    ) -> AppResult<werc_domain::history::HistoryBundle> {
        let path = std::env::current_exe().map_err(|_| werc_domain::AppError::internal())?;
        let executable = path
            .parent()
            .ok_or_else(werc_domain::AppError::internal)?
            .join("werc-native-worker.exe");
        reader::execute(
            &executable,
            option_env!("WERC_WORKER_SHA256").unwrap_or(""),
            request,
        )
    }

    fn stream_database(
        &self,
        request: &werc_domain::history::HistoryReadRequest,
        visitor: &mut dyn FnMut(werc_domain::history::HistoryReadEvent) -> AppResult<()>,
    ) -> AppResult<()> {
        let path = std::env::current_exe().map_err(|_| werc_domain::AppError::internal())?;
        let executable = path
            .parent()
            .ok_or_else(werc_domain::AppError::internal)?
            .join("werc-native-worker.exe");
        reader::execute_stream(
            &executable,
            option_env!("WERC_WORKER_SHA256").unwrap_or(""),
            request,
            visitor,
        )
    }

    fn capabilities(&self) -> Vec<Capability> {
        let mut pending: Vec<_> = [
            (
                "wechat.discover",
                "尚未取得本机微信精确版本与来源路径的授权验证",
            ),
            (
                "keys.acquire",
                "当前平台不支持自动准备微信历史来源",
            ),
            (
                "keys.import",
                "当前平台不支持导入微信来源密钥",
            ),
            (
                "messages.read",
                "微信数据库解密参数、布局与 WAL 一致性尚待验证",
            ),
            ("messages.observe", "尚无经过验证的新消息观测来源"),
            (
                "messages.send_text",
                "尚无已验证后台发送提供方；不会生成假发送任务",
            ),
            ("messages.send_image", "静态PNG原生发送需显式连接受支持通道"),
            ("messages.send_video", "原生视频发送需显式连接受支持通道"),
            ("messages.send_emoticon", "原生表情发送需显式连接受支持通道"),
            ("messages.send_file", "原生文件发送需显式连接受支持通道"),
            ("messages.send_media", "各媒体类型发送尚未实现"),
            (
                "messages.send_voice",
                "音频处理不等于微信原生语音发送；尚无经验证的提交后端",
            ),
            ("media.read", "微信媒体格式与解密尚待适配"),
            ("moments.read", "朋友圈数据适配尚未实现"),
            ("moments.publish", "尚无经过验证的个人朋友圈发布后端"),
            (
                "exports.create",
                "完整导出任务、媒体与更多格式仍未实现；离线消息导出见history.export",
            ),
        ]
        .into_iter()
        .map(|(id, reason)| Capability::pending(id, reason))
        .collect();
        if cfg!(windows) {
            if let Some(acquisition) = pending.iter_mut().find(|c| c.id == "keys.acquire") {
                acquisition.evidence = Evidence::Implemented;
                acquisition.availability = Availability::NeedsSetup;
                acquisition.reason = "连接已登录的受支持微信后，可自动准备本机消息来源；具体结果以来源读取为准".into();
                acquisition.execution_mode = Some("local_read_only".into());
                acquisition.requires_user_action = true;
            }
            if let Some(import) = pending.iter_mut().find(|c| c.id == "keys.import") {
                import.evidence = Evidence::Implemented;
                import.availability = Availability::NeedsSetup;
                import.reason = "绑定本机来源后可手工导入，使用 Windows DPAPI 保存；能否解密由来源读取验证".into();
                import.execution_mode = Some("local".into());
                import.requires_user_action = true;
            }
            pending.push(Capability { id:"wechat.data_sources".into(), evidence:Evidence::Implemented, availability:Availability::NeedsSetup, reason:"显式检测系统文档/手选目录的数据库元数据；不读取内容，不证明账号归属或微信已连接".into(), execution_mode:Some("local".into()), requires_foreground:false, requires_user_action:true });
            pending.push(Capability {
                id: "media.audio_preview".into(),
                evidence: Evidence::Implemented,
                availability: Availability::NeedsSetup,
                reason: "预听用户导入的PCM16 WAV；不是微信音频解码或原生语音发送".into(),
                execution_mode: Some("local".into()),
                requires_foreground: false,
                requires_user_action: true,
            });
            if let Some(discovery) = pending.iter_mut().find(|c| c.id == "wechat.discover") {
                discovery.evidence = Evidence::Implemented;
                discovery.availability = Availability::Ready;
                discovery.requires_user_action = true;
                discovery.reason = "可显式检测安装/进程元数据并读取精确版本；不读取聊天或密钥，不代表微信读写已就绪".into();
            }
            pending.push(Capability {
                id: "keys.protect".into(),
                evidence: Evidence::Implemented,
                availability: Availability::NeedsSetup,
                reason: "本机导入以 DPAPI CurrentUser 保护；密钥正确性和微信数据库连接仍需单独验证"
                    .into(),
                execution_mode: Some("local".into()),
                requires_foreground: false,
                requires_user_action: true,
            });
        }
        pending.push(Capability {
            id: "history.import".into(), evidence: Evidence::Implemented, availability: Availability::NeedsSetup,
            reason: "显式读取离线DB/WAL，已实现SQLCipher4 raw-key固定参数页认证与内存视图；不等于完整SQLCipher引擎或实时微信兼容验收".into(), execution_mode: Some("isolated_process".into()), requires_foreground: false, requires_user_action: true,
        });
        for (id, reason) in [
            (
                "history.read",
                "查询已导入快照的消息、联系人与会话；非实时微信连接",
            ),
            (
                "history.export",
                "导入快照支持消息导出；已保存资料的媒体、归档、计划和其他导出入口见资料库工具",
            ),
        ] {
            pending.push(Capability {
                id: id.into(),
                evidence: Evidence::Implemented,
                availability: Availability::NeedsSetup,
                reason: reason.into(),
                execution_mode: Some("local".into()),
                requires_foreground: false,
                requires_user_action: true,
            });
        }
        pending
    }
    fn discover(&self) -> AppResult<DiscoveryReport> {
        discovery::discover()
    }
    fn inspect(&self, path: &str) -> AppResult<ClientInstallation> {
        discovery::inspect(path)
    }
    fn protect_key(&self, context: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        keys::protect(context, key)
    }
    fn unprotect_key(&self, context: &str, protected: &[u8]) -> AppResult<Vec<u8>> {
        keys::unprotect(context, protected)
    }
}

#[cfg(windows)]
pub mod emoticon_codec;

#[cfg(windows)]
mod sns_media;

#[cfg(windows)]
mod article_renderer;
