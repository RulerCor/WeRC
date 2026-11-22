//! Exact-build native provider with prepared recipients. The historical type name
//! is retained for source compatibility. Explicit construction only;
//! the normal desktop provider is not silently enabled by a DLL's presence.
use sha2::{Digest, Sha256};
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle, process::CommandExt};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Mutex,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use werc_application::ports::CapabilityProvider;
use werc_domain::{AppError, AppResult, outbox::*, receive::*};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::*,
};
use zeroize::Zeroizing;
use crate::native_profiles::{NativeFeature, NativeProfile};

#[path = "native_recipients.rs"]
mod recipients;
#[path = "native_articles.rs"]
mod native_articles;
#[path = "native_settings.rs"]
mod native_settings;

#[path = "native_long_text.rs"]
mod long_text;
#[path = "native_mentions.rs"]
mod mentions;
#[path = "native_moment_action.rs"]
mod moment_action;
#[path = "native_moment_send.rs"]
mod moment_send;
#[path = "native_quote.rs"]
mod quote;
pub const NATIVE_PROTOCOL_VERSION: u32 = 27;
enum DownloadMedia {
    Favorite(u32),
    Image,
    OriginalImage,
    File,
    Video,
}
const PACKET_SIZE: usize = 1528;
const ACCOUNT: usize = 80;
const TARGET: usize = 208;
const ATTEMPT: usize = 336;
const DIGEST: usize = 400;
const TEXT: usize = 465;
struct Packet(Zeroizing<[u8; PACKET_SIZE]>);
impl Packet {
    fn new(op: u32) -> Self {
        let mut p = Self(Zeroizing::new([0; PACKET_SIZE]));
        p.put32(0, PACKET_SIZE as u32);
        p.put32(4, NATIVE_PROTOCOL_VERSION);
        p.put32(8, op);
        p
    }
    fn copy(&self, op: u32) -> Self {
        let mut p = Self(Zeroizing::new(*self.0));
        p.put32(8, op);
        p.put32(12, 0);
        p
    }
    fn put32(&mut self, offset: usize, value: u32) {
        self.0[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(&mut self, offset: usize, value: u64) {
        self.0[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn u32(&self, offset: usize) -> u32 {
        u32::from_le_bytes(self.0[offset..offset + 4].try_into().expect("fixed offset"))
    }
    fn u64(&self, offset: usize) -> u64 {
        u64::from_le_bytes(self.0[offset..offset + 8].try_into().expect("fixed offset"))
    }
    fn text(&mut self, offset: usize, capacity: usize, value: &str) -> AppResult<()> {
        if value.len() >= capacity || value.contains('\0') {
            return Err(error("NATIVE_ARGUMENT_INVALID"));
        }
        self.0[offset..offset + capacity].fill(0);
        self.0[offset..offset + value.len()].copy_from_slice(value.as_bytes());
        Ok(())
    }
    fn string(&self, offset: usize, capacity: usize) -> AppResult<String> {
        let bytes = &self.0[offset..offset + capacity];
        let end = bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| error("NATIVE_PROTOCOL_INVALID"))?;
        String::from_utf8(bytes[..end].to_vec()).map_err(|_| error("NATIVE_PROTOCOL_INVALID"))
    }
}
fn error(code: &str) -> AppError {
    AppError::new(code, "原生消息通道未完成本次操作；不自动重试提交")
}

pub(crate) struct Job(HANDLE);
// A job handle has no thread-affine API and is only closed when transport drops.
unsafe impl Send for Job {}
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Job {
    pub(crate) fn attach(child: &Child) -> AppResult<Self> {
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(error("NATIVE_JOB_UNAVAILABLE"));
            }
            let job = Self(raw);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = 1;
            limits.ProcessMemoryLimit = 128 * 1024 * 1024;
            if SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(error("NATIVE_JOB_UNAVAILABLE"));
            }
            let cpu = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE
                    | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                Anonymous: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 { CpuRate: 2000 },
            };
            if SetInformationJobObject(
                raw,
                JobObjectCpuRateControlInformation,
                &cpu as *const _ as *const _,
                std::mem::size_of_val(&cpu) as u32,
            ) == 0
                || AssignProcessToJobObject(raw, child.as_raw_handle() as HANDLE) == 0
            {
                return Err(error("NATIVE_JOB_UNAVAILABLE"));
            }
            Ok(job)
        }
    }
}
struct Transport {
    child: Child,
    input: Option<ChildStdin>,
    response: Receiver<std::io::Result<Packet>>,
    _job: Job,
    _host_file: File,
    binding: Packet,
    dead: bool,
}
impl Transport {
    fn start(host: &Path, expected: &str, pid: u32, started: u64) -> AppResult<Self> {
        if !host.is_absolute()
            || expected.len() != 64
            || host.file_name().and_then(|s| s.to_str()) != Some("werc-message-host.exe")
        {
            return Err(error("NATIVE_COMPONENT_INVALID"));
        }
        for parent in host.ancestors() {
            use std::os::windows::fs::MetadataExt;
            let metadata =
                std::fs::symlink_metadata(parent).map_err(|_| error("NATIVE_COMPONENT_INVALID"))?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(error("NATIVE_COMPONENT_INVALID"));
            }
        }
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(host)
            .map_err(|_| error("NATIVE_COMPONENT_INVALID"))?;
        if file
            .metadata()
            .map_err(|_| error("NATIVE_COMPONENT_INVALID"))?
            .len()
            > 16 * 1024 * 1024
        {
            return Err(error("NATIVE_COMPONENT_INVALID"));
        }
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|_| error("NATIVE_COMPONENT_INVALID"))?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        if hex::encode(hash.finalize()) != expected {
            return Err(error("NATIVE_COMPONENT_INVALID"));
        }
        let mut child = Command::new(host)
            .current_dir(
                host.parent()
                    .ok_or_else(|| error("NATIVE_COMPONENT_INVALID"))?,
            )
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| error("NATIVE_HOST_UNAVAILABLE"))?;
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let input = child
            .stdin
            .take()
            .ok_or_else(|| error("NATIVE_HOST_UNAVAILABLE"))?;
        let mut output = child
            .stdout
            .take()
            .ok_or_else(|| error("NATIVE_HOST_UNAVAILABLE"))?;
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            loop {
                let mut packet = Packet::new(0);
                let result = output.read_exact(packet.0.as_mut()).map(|_| packet);
                let failed = result.is_err();
                if tx.send(result).is_err() || failed {
                    break;
                }
            }
        });
        let mut transport = Self {
            child,
            input: Some(input),
            response: rx,
            _job: job,
            _host_file: file,
            binding: Packet::new(1),
            dead: false,
        };
        let mut inspect = Packet::new(1);
        inspect.put32(16, pid);
        inspect.put64(24, started);
        let result = transport.exchange(inspect)?;
        let account = result.string(ACCOUNT, 128)?;
        if account.is_empty() || result.u64(24) == 0 || result.u32(20) != transport.child.id() {
            return Err(error("NATIVE_IDENTITY_UNVERIFIED"));
        }
        transport.binding = result.copy(2);
        getrandom::fill(&mut transport.binding.0[48..80])
            .map_err(|_| error("NATIVE_RANDOM_FAILED"))?;
        let result = transport.exchange(transport.binding.copy(2))?;
        transport.binding = result.copy(0);
        // Binding never retains a body, attempt or digest after any request.
        transport.binding.0[TARGET..1492].fill(0);
        Ok(transport)
    }
    fn exchange(&mut self, request: Packet) -> AppResult<Packet> {
        if self.dead {
            return Err(error("NATIVE_DISCONNECTED"));
        }
        if self
            .input
            .as_mut()
            .ok_or_else(|| error("NATIVE_DISCONNECTED"))?
            .write_all(request.0.as_ref())
            .is_err()
        {
            self.dead = true;
            return Err(error("NATIVE_DISCONNECTED"));
        }
        let result = match self.response.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(p)) => p,
            _ => {
                self.dead = true;
                self.input.take();
                let _ = self.child.kill();
                return Err(error("NATIVE_RESULT_UNKNOWN"));
            }
        };
        if result.u32(0) != PACKET_SIZE as u32
            || result.u32(4) != NATIVE_PROTOCOL_VERSION
            || result.u32(8) != request.u32(8)
            || result.u32(16) != request.u32(16)
        {
            self.dead = true;
            return Err(error("NATIVE_PROTOCOL_INVALID"));
        }
        if result.u32(12) != 0 {
            if matches!(request.u32(8),70|71) && result.u32(12)==50 {
                return Err(AppError::new("WECHAT_SETTINGS_UNAVAILABLE","此精确构建没有已验证的后台原生设置能力"));
            }
            if matches!(request.u32(8), 1 | 2) && result.u32(12) == 1245 {
                return Err(AppError::new("NATIVE_LOGIN_REQUIRED", "请先在微信登录同一账号"));
            }
            if request.u32(8) == 1 {
                let detail = match result.u32(12) {
                    1306 => Some((
                        "NATIVE_BUILD_UNSUPPORTED",
                        "微信DLL不是已适配的原版或已知防撤回补丁版。保留现有安装，请查看支持范围",
                    )),
                    13 => Some((
                        "NATIVE_ENTRY_CONFLICT",
                        "微信内存中的关键调用入口与已识别DLL不一致，可能存在其他插件冲突；尚未连接或发送",
                    )),
                    123 => Some((
                        "NATIVE_INSTALL_LAYOUT",
                        "已加载的微信DLL不在当前客户端安装位置，无法确定配套关系",
                    )),
                    5 => Some((
                        "NATIVE_ACCESS_DENIED",
                        "无法访问当前微信进程；请让WeRC与微信使用同一Windows用户和匹配的权限级别",
                    )),
                    577 => Some((
                        "NATIVE_COMPONENT_HASH_MISMATCH",
                        "WeRC原生组件与本次源码构建不匹配，请使用同一构建的配套组件",
                    )),
                    170 => Some((
                        "NATIVE_BUSY",
                        "当前微信已有一个收发连接，请关闭旧连接后重试",
                    )),
                    _ => None,
                };
                if let Some((code, message)) = detail {
                    return Err(AppError::new(code, message));
                }
            }
            return Err(AppError::new(
                "NATIVE_CALL_REJECTED",
                &format!("原生接口拒绝，Windows 状态 {}；未自动重试", result.u32(12)),
            ));
        }
        Ok(result)
    }
    fn check_identity(&mut self) -> AppResult<()> {
        let result = self.exchange(self.binding.copy(1))?;
        if result.0[ACCOUNT..ACCOUNT + 128] != self.binding.0[ACCOUNT..ACCOUNT + 128]
            || result.u64(24) != self.binding.u64(24)
        {
            self.dead = true;
            return Err(error("NATIVE_IDENTITY_CHANGED"));
        }
        Ok(())
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Err(_) => break,
                _ => {}
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}
pub struct NativeFileHelperProvider {
    profile: &'static NativeProfile,
    account_id: String,
    context: SendContext,
    transport: Mutex<Transport>,
    codec: Option<(std::path::PathBuf, String)>,
    closing: std::sync::atomic::AtomicBool,
    download_guard: Mutex<()>,
    recipients: Mutex<Vec<recipients::RecipientLease>>,
}
impl NativeFileHelperProvider {
    pub(crate) fn native_profile(&self) -> &'static NativeProfile { self.profile }
    fn require_feature(&self, feature: NativeFeature) -> AppResult<()> {
        self.profile.require(feature)
    }
    pub(crate) fn request_disconnect(&self) {
        self.closing
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub(crate) fn require_open(&self) -> AppResult<()> {
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            Err(error("NATIVE_DISCONNECTED"))
        } else {
            Ok(())
        }
    }
    // Keep only one request/response under the transport lock. Native download
    // work runs asynchronously; receive/ACK can proceed between polls/chunks.
    fn download_exchange(&self, request: Packet) -> AppResult<Packet> {
        self.require_open()?;
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        self.require_open()?;
        channel.exchange(request)
    }
    fn download_image_bytes(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.download_media_bytes(context, descriptor, DownloadMedia::Image)
    }
    fn download_media_bytes(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
        kind: DownloadMedia,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        if matches!(kind, DownloadMedia::Favorite(_)) {
            self.require_feature(NativeFeature::FavoriteDownload)?;
        }
        let (operation, media_type, exact_size) = match kind {
            DownloadMedia::Image => (22, 2u32, false),
            DownloadMedia::OriginalImage => (49, 1u32, true),
            DownloadMedia::File => (27, 5, true),
            DownloadMedia::Video => (28, 4, false),
            DownloadMedia::Favorite(2) => (57, 2, false),
            DownloadMedia::Favorite(3 | 8) => (57, 5, true),
            DownloadMedia::Favorite(4) => (57, 4, false),
            DownloadMedia::Favorite(_) => return Err(error("FAVORITE_MEDIA_UNSUPPORTED")),
        };
        let maximum = if media_type == 4 {
            werc_domain::received_media::RECEIVED_VIDEO_MAX_BYTES
        } else {
            werc_domain::received_media::RECEIVED_MEDIA_MAX_BYTES
        };
        let _download = self
            .download_guard
            .try_lock()
            .map_err(|_| error("NATIVE_DOWNLOAD_BUSY"))?;
        self.require_open()?;
        if context != &self.context || !(1..=maximum).contains(&descriptor.declared_size) {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        let mut request = {
            let mut channel = self
                .transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?;
            self.require_open()?;
            channel.check_identity()?;
            channel_binding(&channel, operation)
        };
        request.text(TARGET, 128, "filehelper")?;
        let mut attempt = [0u8; 16];
        getrandom::fill(&mut attempt).map_err(|_| error("NATIVE_RANDOM_FAILED"))?;
        request.text(ATTEMPT, 64, &hex::encode(attempt))?;
        let mut digest = Sha256::new();
        digest.update(descriptor.file_key.as_bytes());
        digest.update(descriptor.aes_key.as_bytes());
        digest.update(descriptor.declared_size.to_le_bytes());
        digest.update(media_type.to_le_bytes());
        digest.update(operation.to_le_bytes());
        request.text(DIGEST, 65, &hex::encode(digest.finalize()))?;
        request.text(TEXT, 769, &descriptor.file_key)?;
        request.text(TEXT + 769, 33, &descriptor.aes_key)?;
        request.put64(1520, descriptor.declared_size);
        request.put32(1500, media_type);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        request.put64(40, now + 90);
        self.download_exchange(request.copy(operation))?;
        request.0[TEXT..TEXT + 1024].fill(0);
        let result = (|| {
            let until = Instant::now() + Duration::from_secs(60);
            let actual_size = loop {
                let status = self.download_exchange(request.copy(23))?;
                match status.u32(1492) {
                    2 => {
                        let size = status.u64(1520);
                        if !(1..=maximum).contains(&size) {
                            return Err(error("NATIVE_PROTOCOL_INVALID"));
                        }
                        if exact_size && size != descriptor.declared_size {
                            return Err(error("NATIVE_PROTOCOL_INVALID"));
                        }
                        break size as usize;
                    }
                    3 => {
                        return Err(AppError::new(
                            "NATIVE_DOWNLOAD_FAILED",
                            &format!(
                                "媒体下载失败，状态{}，原生状态{}，阶段{}，完成字节数{}",
                                status.u32(1500),
                                status.u32(1496),
                                status.u64(1512),
                                status.u64(40)
                            ),
                        ));
                    }
                    1 if Instant::now() < until && status.u64(1520) == descriptor.declared_size => {
                        std::thread::sleep(Duration::from_millis(100))
                    }
                    _ => return Err(error("NATIVE_DOWNLOAD_TIMEOUT")),
                }
            };
            let mut bytes = Zeroizing::new(Vec::with_capacity(actual_size));
            while bytes.len() < actual_size {
                let mut chunk = request.copy(24);
                chunk.put64(1512, bytes.len() as u64);
                let response = self.download_exchange(chunk)?;
                let n = response.u32(1496) as usize;
                if response.u32(1492) != 2
                    || response.u64(1512) != bytes.len() as u64
                    || response.u64(1520) != actual_size as u64
                    || n == 0
                    || n > 1024
                    || n > actual_size - bytes.len()
                {
                    return Err(error("NATIVE_PROTOCOL_INVALID"));
                }
                bytes.extend_from_slice(&response.0[TEXT..TEXT + n]);
            }
            self.require_open()?;
            self.transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?
                .check_identity()?;
            Ok(bytes)
        })();
        // Release is allowed while closing; disconnect waits for this shared
        // provider lease before dropping the host and unloading the bridge.
        let released = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?
            .exchange(request.copy(25));
        match result {
            Ok(bytes) => {
                released?;
                Ok(bytes)
            }
            Err(e) => Err(e),
        }
    }
    /// Only explicit local application assembly may choose these arguments.
    /// `account_id` is the existing product account key, never a replacement wxid.
    pub fn connect(
        account_id: &str,
        host: &Path,
        host_sha256: &str,
        pid: u32,
        installation_path: &str,
    ) -> AppResult<Self> {
        let installation = crate::discovery::inspect(installation_path)?;
        let (actual, started) = process_identity(pid)?;
        if Path::new(&actual).canonicalize().ok()
            != Path::new(installation_path).canonicalize().ok()
        {
            return Err(error("NATIVE_INSTALLATION_MISMATCH"));
        }
        let profile = crate::native_profiles::check_process(pid)?;
        let transport = Transport::start(host, host_sha256, pid, started)?;
        let sender = transport.binding.string(ACCOUNT, 128)?;
        let epoch = hex::encode(Sha256::digest(
            [&transport.binding.0[16..40], sender.as_bytes()].concat(),
        ));
        let context = SendContext {
            sender_id: sender,
            target_id: "filehelper".into(),
            target_display_name: "文件传输助手".into(),
            identity_verified: true,
            target_verified: true,
            epoch,
            installation_sha256: installation.sha256,
            adapter_sha256: host_sha256.into(),
            execution_mode: SendExecutionMode::Native,
        };
        Ok(Self {
            profile,
            account_id: account_id.into(),
            context,
            transport: Mutex::new(transport),
            codec: None,
            closing: std::sync::atomic::AtomicBool::new(false),
            download_guard: Mutex::new(()),
            recipients: Mutex::new(Vec::new()),
        })
    }
    pub fn with_voice_codec(mut self, path: std::path::PathBuf, hash: String) -> Self {
        self.codec = Some((path, hash));
        self
    }
    fn upload_image(
        &self,
        s: &TextSubmission<'_>,
        image: &ImageSendAsset,
        bytes: &[u8],
        probe: bool,
    ) -> AppResult<u32> {
        let (info, canonical) = werc_domain::image::canonical_png(bytes)?;
        if *s.context != self.resolve_send_context(&self.account_id, &s.context.target_id)?
            || !s.text.is_empty()
            || canonical.as_slice() != bytes
            || info.width != image.width
            || info.height != image.height
            || hex::encode(Sha256::digest(bytes)) != image.png_sha256
        {
            return Err(error("NATIVE_IMAGE_INVALID"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        if s.deadline_unix <= now as i64 || s.deadline_unix > now as i64 + 300 {
            return Err(error("NATIVE_APPROVAL_EXPIRED"));
        }
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut p = channel_binding(&channel, 13);
        p.text(TARGET, 128, &s.context.target_id)?;
        p.text(ATTEMPT, 64, s.attempt_id)?;
        p.text(DIGEST, 65, s.request_digest)?;
        p.text(TEXT, 1024, &image.png_sha256)?;
        p.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        p.put64(1520, bytes.len() as u64);
        p.put32(1500, image.width | (image.height << 16));
        channel.exchange(p.copy(13))?;
        for (i, chunk) in bytes.chunks(1024).enumerate() {
            let mut next = p.copy(14);
            next.0[TEXT..TEXT + 1024].fill(0);
            next.0[TEXT..TEXT + chunk.len()].copy_from_slice(chunk);
            next.put64(1512, (i * 1024) as u64);
            next.put32(1496, chunk.len() as u32);
            channel.exchange(next)?;
        }
        Ok(channel
            .exchange(p.copy(if probe { 15 } else { 16 }))?
            .u32(1492))
    }
    /// Constructs/destroys the native image object; never calls native submit.
    pub fn probe_image(&self, bytes: &[u8]) -> AppResult<bool> {
        self.probe_image_to("filehelper", bytes)
    }
    /// Constructs the selected recipient object only; never submits a message.
    pub fn probe_image_to(&self, target: &str, bytes: &[u8]) -> AppResult<bool> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        let (info, canonical) = werc_domain::image::canonical_png(bytes)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = hex::encode(random);
        let hash = hex::encode(Sha256::digest(&canonical));
        let descriptor = ImageSendAsset {
            asset_id: String::new(),
            source_sha256: String::new(),
            png_sha256: hash.clone(),
            width: info.width,
            height: info.height,
        };
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs() as i64
            + 30;
        let submission = TextSubmission {
            outbox_id: "probe",
            attempt_id: &attempt,
            request_digest: &hash,
            context_digest: &hash,
            context: &context,
            text: "",
            deadline_unix: deadline,
        };
        let mut code = self.upload_image(&submission, &descriptor, &canonical, true)?;
        let until = Instant::now() + Duration::from_secs(20);
        while code == 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
            let mut channel = self
                .transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?;
            let mut p = channel_binding(&channel, 5);
            p.text(TARGET, 128, &context.target_id)?;
            p.text(ATTEMPT, 64, &attempt)?;
            code = channel.exchange(p)?.u32(1492);
        }
        Ok(code == 8)
    }
    fn upload_video(
        &self,
        s: &TextSubmission<'_>,
        video: &VideoSendAsset,
        bytes: &[u8],
        probe: bool,
    ) -> AppResult<u32> {
        let file = &video.file;
        if werc_domain::video::inspect_send_mp4(&file.file_name, bytes)? != video.info {
            return Err(error("NATIVE_VIDEO_INVALID"));
        }
        if *s.context != self.resolve_send_context(&self.account_id, &s.context.target_id)?
            || !s.text.is_empty()
            || bytes.is_empty()
            || bytes.len() > werc_domain::file::MAX_FILE_BYTES
            || bytes.len() != file.size
            || hex::encode(Sha256::digest(bytes)) != file.sha256
        {
            return Err(error("NATIVE_VIDEO_INVALID"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        if s.deadline_unix <= now as i64 || s.deadline_unix > now as i64 + 300 {
            return Err(error("NATIVE_APPROVAL_EXPIRED"));
        }
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut p = channel_binding(&channel, 29);
        p.text(TARGET, 128, &s.context.target_id)?;
        p.text(ATTEMPT, 64, s.attempt_id)?;
        p.text(DIGEST, 65, s.request_digest)?;
        p.text(TEXT, 65, &file.sha256)?;
        p.text(TEXT + 65, 121, &file.file_name)?;
        p.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        p.put64(1520, bytes.len() as u64);
        p.put32(1500, video.info.duration_ms as u32);
        channel.exchange(p.copy(29))?;
        for (i, chunk) in bytes.chunks(1024).enumerate() {
            let mut next = p.copy(30);
            next.0[TEXT..TEXT + 1024].fill(0);
            next.0[TEXT..TEXT + chunk.len()].copy_from_slice(chunk);
            next.put64(1512, (i * 1024) as u64);
            next.put32(1496, chunk.len() as u32);
            channel.exchange(next)?;
        }
        Ok(channel
            .exchange(p.copy(if probe { 31 } else { 32 }))?
            .u32(1492))
    }
    /// Constructs/destroys the native video object and removes its owned staged
    /// copy. The returned duration comes from the client's own media reader.
    pub fn probe_video(&self, name: &str, bytes: &[u8]) -> AppResult<(bool, u32)> {
        self.probe_video_to("filehelper", name, bytes)
    }
    /// Constructs the selected recipient object only; never submits a message.
    pub fn probe_video_to(&self, target: &str, name: &str, bytes: &[u8]) -> AppResult<(bool, u32)> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        werc_domain::file::validate_file_name(name)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = hex::encode(random);
        let hash = hex::encode(Sha256::digest(bytes));
        let descriptor = FileSendAsset {
            asset_id: String::new(),
            sha256: hash.clone(),
            file_name: name.into(),
            size: bytes.len(),
        };
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs() as i64
            + 30;
        let submission = TextSubmission {
            outbox_id: "probe",
            attempt_id: &attempt,
            request_digest: &hash,
            context_digest: &hash,
            context: &context,
            text: "",
            deadline_unix: deadline,
        };
        let video = VideoSendAsset {
            file: descriptor,
            info: werc_domain::video::inspect_send_mp4(name, bytes)?,
        };
        self.upload_video(&submission, &video, bytes, true)?;
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let mut channel = self
                .transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?;
            let mut p = channel_binding(&channel, 5);
            p.text(TARGET, 128, &context.target_id)?;
            p.text(ATTEMPT, 64, &attempt)?;
            let response = channel.exchange(p)?;
            let code = response.u32(1492);
            if code != 1 || Instant::now() >= until {
                return Ok((code == 12, response.u32(1500)));
            }
            drop(channel);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn upload_emoticon(
        &self,
        s: &TextSubmission<'_>,
        emoticon: &EmoticonSendAsset,
        bytes: &[u8],
        probe: bool,
    ) -> AppResult<u32> {
        let file = &emoticon.file;
        if werc_domain::emoticon::inspect_gif(&file.file_name, bytes)? != emoticon.info {
            return Err(error("NATIVE_EMOTICON_INVALID"));
        }
        if *s.context != self.resolve_send_context(&self.account_id, &s.context.target_id)?
            || !s.text.is_empty()
            || bytes.is_empty()
            || bytes.len() > werc_domain::emoticon::MAX_EMOTICON_BYTES
            || bytes.len() != file.size
            || hex::encode(Sha256::digest(bytes)) != file.sha256
        {
            return Err(error("NATIVE_EMOTICON_INVALID"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        if s.deadline_unix <= now as i64 || s.deadline_unix > now as i64 + 300 {
            return Err(error("NATIVE_APPROVAL_EXPIRED"));
        }
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut p = channel_binding(&channel, 33);
        p.text(TARGET, 128, &s.context.target_id)?;
        p.text(ATTEMPT, 64, s.attempt_id)?;
        p.text(DIGEST, 65, s.request_digest)?;
        p.text(TEXT, 65, &file.sha256)?;
        p.text(TEXT + 65, 121, &file.file_name)?;
        p.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        p.put64(1520, bytes.len() as u64);
        p.put32(
            1500,
            u32::from(emoticon.info.width) | (u32::from(emoticon.info.height) << 16),
        );
        channel.exchange(p.copy(33))?;
        for (i, chunk) in bytes.chunks(1024).enumerate() {
            let mut next = p.copy(34);
            next.0[TEXT..TEXT + 1024].fill(0);
            next.0[TEXT..TEXT + chunk.len()].copy_from_slice(chunk);
            next.put64(1512, (i * 1024) as u64);
            next.put32(1496, chunk.len() as u32);
            channel.exchange(next)?;
        }
        Ok(channel
            .exchange(p.copy(if probe { 35 } else { 36 }))?
            .u32(1492))
    }
    /// Constructs/destroys the native emoticon object and removes its owned staged
    /// copy. The client's own GIF decoder must agree on both dimensions.
    pub fn probe_emoticon(&self, name: &str, bytes: &[u8]) -> AppResult<bool> {
        self.probe_emoticon_to("filehelper", name, bytes)
    }
    /// Constructs the selected recipient object only; never submits a message.
    pub fn probe_emoticon_to(&self, target: &str, name: &str, bytes: &[u8]) -> AppResult<bool> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        werc_domain::file::validate_file_name(name)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = hex::encode(random);
        let hash = hex::encode(Sha256::digest(bytes));
        let descriptor = FileSendAsset {
            asset_id: String::new(),
            sha256: hash.clone(),
            file_name: name.into(),
            size: bytes.len(),
        };
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs() as i64
            + 30;
        let submission = TextSubmission {
            outbox_id: "probe",
            attempt_id: &attempt,
            request_digest: &hash,
            context_digest: &hash,
            context: &context,
            text: "",
            deadline_unix: deadline,
        };
        let emoticon = EmoticonSendAsset {
            file: descriptor,
            info: werc_domain::emoticon::inspect_gif(name, bytes)?,
        };
        self.upload_emoticon(&submission, &emoticon, bytes, true)?;
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            let mut channel = self
                .transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?;
            let mut p = channel_binding(&channel, 5);
            p.text(TARGET, 128, &context.target_id)?;
            p.text(ATTEMPT, 64, &attempt)?;
            let response = channel.exchange(p)?;
            let code = response.u32(1492);
            if code != 1 || Instant::now() >= until {
                return Ok(code == 14);
            }
            drop(channel);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn upload_file(
        &self,
        s: &TextSubmission<'_>,
        file: &FileSendAsset,
        bytes: &[u8],
        probe: bool,
    ) -> AppResult<u32> {
        werc_domain::file::validate_file_name(&file.file_name)?;
        if *s.context != self.resolve_send_context(&self.account_id, &s.context.target_id)?
            || !s.text.is_empty()
            || bytes.is_empty()
            || bytes.len() > werc_domain::file::MAX_FILE_BYTES
            || bytes.len() != file.size
            || hex::encode(Sha256::digest(bytes)) != file.sha256
        {
            return Err(error("NATIVE_FILE_INVALID"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        if s.deadline_unix <= now as i64 || s.deadline_unix > now as i64 + 300 {
            return Err(error("NATIVE_APPROVAL_EXPIRED"));
        }
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut p = channel_binding(&channel, 17);
        p.text(TARGET, 128, &s.context.target_id)?;
        p.text(ATTEMPT, 64, s.attempt_id)?;
        p.text(DIGEST, 65, s.request_digest)?;
        p.text(TEXT, 65, &file.sha256)?;
        p.text(TEXT + 65, 121, &file.file_name)?;
        p.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        p.put64(1520, bytes.len() as u64);
        channel.exchange(p.copy(17))?;
        for (i, chunk) in bytes.chunks(1024).enumerate() {
            let mut next = p.copy(18);
            next.0[TEXT..TEXT + 1024].fill(0);
            next.0[TEXT..TEXT + chunk.len()].copy_from_slice(chunk);
            next.put64(1512, (i * 1024) as u64);
            next.put32(1496, chunk.len() as u32);
            channel.exchange(next)?;
        }
        Ok(channel
            .exchange(p.copy(if probe { 19 } else { 20 }))?
            .u32(1492))
    }
    /// Constructs/destroys the native file object; never calls native submit.
    pub fn probe_file(&self, name: &str, bytes: &[u8]) -> AppResult<bool> {
        self.probe_file_to("filehelper", name, bytes)
    }
    /// Constructs the selected recipient object only; never submits a message.
    pub fn probe_file_to(&self, target: &str, name: &str, bytes: &[u8]) -> AppResult<bool> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        werc_domain::file::validate_file_name(name)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = hex::encode(random);
        let hash = hex::encode(Sha256::digest(bytes));
        let descriptor = FileSendAsset {
            asset_id: String::new(),
            sha256: hash.clone(),
            file_name: name.into(),
            size: bytes.len(),
        };
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs() as i64
            + 30;
        let submission = TextSubmission {
            outbox_id: "probe",
            attempt_id: &attempt,
            request_digest: &hash,
            context_digest: &hash,
            context: &context,
            text: "",
            deadline_unix: deadline,
        };
        let mut code = self.upload_file(&submission, &descriptor, bytes, true)?;
        let until = Instant::now() + Duration::from_secs(20);
        while code == 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
            let mut channel = self
                .transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?;
            let mut p = channel_binding(&channel, 5);
            p.text(TARGET, 128, &context.target_id)?;
            p.text(ATTEMPT, 64, &attempt)?;
            code = channel.exchange(p)?.u32(1492);
        }
        Ok(code == 10)
    }
    fn upload_voice(
        &self,
        s: &TextSubmission<'_>,
        voice: &VoiceSendAsset,
        bytes: &[u8],
        probe: bool,
    ) -> AppResult<u32> {
        if *s.context != self.resolve_send_context(&self.account_id, &s.context.target_id)?
            || !s.text.is_empty()
            || werc_domain::voice::silk_duration(bytes)? != voice.duration_ms
            || hex::encode(Sha256::digest(bytes)) != voice.silk_sha256
        {
            return Err(error("NATIVE_VOICE_INVALID"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        if s.deadline_unix <= now as i64 || s.deadline_unix > now as i64 + 300 {
            return Err(error("NATIVE_APPROVAL_EXPIRED"));
        }
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut p = channel_binding(&channel, 9);
        p.text(TARGET, 128, &s.context.target_id)?;
        p.text(ATTEMPT, 64, s.attempt_id)?;
        p.text(DIGEST, 65, s.request_digest)?;
        p.text(TEXT, 1024, &voice.silk_sha256)?;
        p.put64(40, self.send_deadline(s.context, s.deadline_unix, now)?);
        p.put64(1520, bytes.len() as u64);
        p.put32(1500, voice.duration_ms);
        channel.exchange(p.copy(9))?;
        for (i, chunk) in bytes.chunks(1024).enumerate() {
            let mut next = p.copy(10);
            next.0[TEXT..TEXT + 1024].fill(0);
            next.0[TEXT..TEXT + chunk.len()].copy_from_slice(chunk);
            next.put64(1512, (i * 1024) as u64);
            next.put32(1496, chunk.len() as u32);
            channel.exchange(next)?;
        }
        Ok(channel
            .exchange(p.copy(if probe { 11 } else { 12 }))?
            .u32(1492))
    }
    /// Executes only the native voice constructor and destroys the object. No submit call.
    pub fn probe_voice(&self, voice: &werc_domain::voice::EncodedVoice) -> AppResult<bool> {
        self.probe_voice_to("filehelper", voice)
    }
    /// Constructs the selected recipient object only; never submits a message.
    pub fn probe_voice_to(
        &self,
        target: &str,
        voice: &werc_domain::voice::EncodedVoice,
    ) -> AppResult<bool> {
        let context = self.resolve_send_context(&self.account_id, target)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| AppError::internal())?;
        let attempt = hex::encode(random);
        let hash = hex::encode(Sha256::digest(&voice.bytes));
        let descriptor = VoiceSendAsset {
            asset_id: String::new(),
            source_sha256: String::new(),
            silk_sha256: hash.clone(),
            codec_sha256: voice.codec_sha256.clone(),
            duration_ms: voice.duration_ms,
        };
        let deadline = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs() as i64
            + 30;
        let submission = TextSubmission {
            outbox_id: "probe",
            attempt_id: &attempt,
            request_digest: &hash,
            context_digest: &hash,
            context: &context,
            text: "",
            deadline_unix: deadline,
        };
        let mut code = self.upload_voice(&submission, &descriptor, &voice.bytes, true)?;
        let until = Instant::now() + Duration::from_secs(20);
        while code == 1 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
            let mut channel = self
                .transport
                .lock()
                .map_err(|_| error("NATIVE_DISCONNECTED"))?;
            let mut p = channel_binding(&channel, 5);
            p.text(TARGET, 128, &context.target_id)?;
            p.text(ATTEMPT, 64, &attempt)?;
            code = channel.exchange(p)?.u32(1492);
        }
        Ok(code == 6)
    }
    fn poll_native_messages(&self) -> AppResult<ReceiveBatch> {
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        let packet = channel_binding(&channel, 4);
        let result = channel.exchange(packet)?;
        if result.u32(1496) > 1 {
            return Err(error("NATIVE_PROTOCOL_INVALID"));
        }
        let route = if result.u32(1496) == 1 {
            let route = ReceivedRoute {
                sender: result.string(TEXT, 128)?,
                receiver: result.string(TEXT + 128, 128)?,
                conversation: result.string(TARGET, 128)?,
            };
            if !route.valid_for(&self.context.sender_id) {
                return Err(error("NATIVE_PROTOCOL_INVALID"));
            }
            Some(route)
        } else {
            None
        };
        let message = if result.u32(1496) == 0 {
            None
        } else {
            let kind = result.u32(1492);
            let length =
                usize::try_from(result.u64(40)).map_err(|_| error("NATIVE_PROTOCOL_INVALID"))?;
            if !observed_type(kind) || length == 0 || length > 16383 {
                return Err(error("NATIVE_PROTOCOL_INVALID"));
            }
            let sequence = result.u64(1512);
            let native_id = result.u64(1520);
            let mut bytes = Zeroizing::new(Vec::with_capacity(length));
            while bytes.len() < length {
                let mut chunk = channel_binding(&channel, 21);
                chunk.put64(1512, sequence);
                chunk.put32(1500, bytes.len() as u32);
                let chunk = channel.exchange(chunk)?;
                let count = chunk.u32(1496) as usize;
                if count != (length - bytes.len()).min(1024)
                    || chunk.u64(1512) != sequence
                    || chunk.u64(1520) != native_id
                    || chunk.u64(40) != length as u64
                    || chunk.u32(1492) != kind
                {
                    return Err(error("NATIVE_PROTOCOL_INVALID"));
                }
                bytes.extend_from_slice(&chunk.0[TEXT..TEXT + count]);
            }
            let content = std::str::from_utf8(&bytes)
                .map_err(|_| error("NATIVE_PROTOCOL_INVALID"))?
                .to_owned();
            let (text, metadata) = if kind == 1 {
                (content, None)
            } else {
                (String::new(), Some(content))
            };
            Some(ReceivedText {
                sequence,
                native_message_id: native_id.to_string(),
                text,
                message_type: kind,
                metadata,
            })
        };
        let voice_length = result.u32(1504) as usize;
        let inline_voice = if voice_length == 0 {
            None
        } else {
            if voice_length > 262144 || message.as_ref().is_none_or(|m| m.message_type != 34) {
                return Err(error("NATIVE_PROTOCOL_INVALID"));
            }
            let mut bytes = Zeroizing::new(Vec::with_capacity(voice_length));
            while bytes.len() < voice_length {
                let mut chunk = channel_binding(&channel, 26);
                chunk.put64(1512, result.u64(1512));
                chunk.put32(1500, bytes.len() as u32);
                let chunk = channel.exchange(chunk)?;
                let count = chunk.u32(1496) as usize;
                if count != (voice_length - bytes.len()).min(1024)
                    || chunk.u32(1492) != 34
                    || chunk.u64(1512) != result.u64(1512)
                    || chunk.u64(1520) != result.u64(1520)
                    || chunk.u64(40) != voice_length as u64
                {
                    return Err(error("NATIVE_PROTOCOL_INVALID"));
                }
                bytes.extend_from_slice(&chunk.0[TEXT..TEXT + count]);
            }
            Some(bytes)
        };
        Ok(ReceiveBatch {
            route,
            inline_voice,
            context: self.context.clone(),
            dropped: result.u32(1500),
            message,
        })
    }
    /// Read-only diagnostic for exact outgoing object identity. A nonzero ID
    /// remains an observation until independently looked up and authenticated.
    pub fn observed_text_message_id(
        &self,
        context: &SendContext,
        attempt: &str,
    ) -> AppResult<(Option<u64>, u32)> {
        self.receipt_context(&self.account_id, context)?;
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        let mut packet = channel_binding(&channel, 5);
        packet.text(TARGET, 128, &context.target_id)?;
        packet.text(ATTEMPT, 64, attempt)?;
        let result = channel.exchange(packet)?;
        let id = result.u64(1512);
        Ok((
            (result.u32(1492) == 2 && id != 0).then_some(id),
            result.u32(1504),
        ))
    }
    /// Research builds alone populate this test-owned task metadata. No status
    /// transition or delivery claim is derived from these unverified fields.
    pub fn observed_text_task_fields(
        &self,
        context: &SendContext,
        attempt: &str,
    ) -> AppResult<serde_json::Value> {
        self.receipt_context(&self.account_id, context)?;
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        let mut packet = channel_binding(&channel, 5);
        packet.text(TARGET, 128, &context.target_id)?;
        packet.text(ATTEMPT, 64, attempt)?;
        let result = channel.exchange(packet)?;
        let value = result.string(TEXT, 1024)?;
        if value.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&value).map_err(|_| error("NATIVE_PROTOCOL_INVALID"))
    }
    fn receipt(&self, id: &str, attempt: &str, digest: &str, evidence: u32) -> SendReceipt {
        SendReceipt {
            outbox_id: id.into(),
            attempt_id: attempt.into(),
            context_digest: digest.into(),
            evidence: if evidence == 4 {
                SendEvidence::RejectedBeforeSubmission
            } else if evidence == 1
                || evidence == 2
                || evidence == 5
                || evidence == 7
                || evidence == 9
                || evidence == 11
                || evidence == 13
            {
                SendEvidence::NativeQueued
            } else {
                SendEvidence::Unknown
            },
            provider_reference: Some(if evidence == 13 {
                "native-emoticon-return:0".into()
            } else if evidence == 11 {
                "native-video-return:0".into()
            } else if evidence == 9 {
                "native-file-return:0".into()
            } else if evidence == 7 {
                "native-image-return:0".into()
            } else if evidence == 5 {
                "native-voice-return:0".into()
            } else {
                format!("native-callback:{evidence}")
            }),
        }
    }
}
fn channel_binding(channel: &Transport, op: u32) -> Packet {
    channel.binding.copy(op)
}
fn process_identity(pid: u32) -> AppResult<(String, u64)> {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::*};
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(error("NATIVE_PROCESS_UNAVAILABLE"));
        }
        let mut name = [0u16; 32768];
        let mut size = name.len() as u32;
        let mut created: FILETIME = std::mem::zeroed();
        let mut exit = created;
        let mut kernel = created;
        let mut user = created;
        let valid = QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size) != 0
            && GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user) != 0;
        CloseHandle(process);
        if !valid {
            return Err(error("NATIVE_PROCESS_UNAVAILABLE"));
        }
        let path = String::from_utf16(&name[..size as usize])
            .map_err(|_| error("NATIVE_PROCESS_UNAVAILABLE"))?;
        Ok((
            path,
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        ))
    }
}
impl CapabilityProvider for NativeFileHelperProvider {
    fn wechat_settings_read(&self,a:&str)->AppResult<werc_domain::wechat_settings::SettingsSnapshot>{self.settings_read(a)}
    fn wechat_settings_apply(&self,a:&str,t:&werc_domain::wechat_settings::SettingTarget,e:&str)->AppResult<werc_domain::wechat_settings::SettingResult>{self.settings_apply(a,t,e)}
    fn article_history(&self,account:&str,publisher:&str,cursor:Option<&str>,cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::article_history::ArticleHistoryPage>{NativeFileHelperProvider::article_history(self,account,publisher,cursor,cancel)}
    fn fetch_article(&self,account:&str,url:&str,cancel:&std::sync::atomic::AtomicBool)->AppResult<werc_domain::articles::Article>{self.article_body(account,url,cancel)}

    fn download_favorite_media(
        &self,
        context: &SendContext,
        kind: u32,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.download_media_bytes(context, descriptor, DownloadMedia::Favorite(kind))
    }

    fn download_received_video(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedVideoDescriptor,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.download_media_bytes(context, &descriptor.media, DownloadMedia::Video)
    }
    fn download_received_file(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.download_media_bytes(context, descriptor, DownloadMedia::File)
    }
    fn download_original_image(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.download_media_bytes(context, descriptor, DownloadMedia::OriginalImage)
    }
    fn download_received_image(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.download_image_bytes(context, descriptor)
    }
    fn submit_image(&self, s: ImageSubmission<'_>) -> AppResult<SendReceipt> {
        let result = self.upload_image(&s.submission, s.image, s.png, false)?;
        Ok(self.receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            result,
        ))
    }
    fn reconcile_image(
        &self,
        q: TextReceiptQuery<'_>,
        reference: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.reconcile_text(q, reference)
    }

    fn submit_video(&self, s: VideoSubmission<'_>) -> AppResult<SendReceipt> {
        let result = self.upload_video(&s.submission, s.video, s.bytes, false)?;
        Ok(self.receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            result,
        ))
    }
    fn reconcile_video(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.reconcile_text(q, r)
    }
    fn submit_emoticon(&self, s: EmoticonSubmission<'_>) -> AppResult<SendReceipt> {
        let result = self.upload_emoticon(&s.submission, s.emoticon, s.bytes, false)?;
        Ok(self.receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            result,
        ))
    }
    fn reconcile_emoticon(
        &self,
        q: TextReceiptQuery<'_>,
        r: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.reconcile_text(q, r)
    }
    fn submit_file(&self, s: FileSubmission<'_>) -> AppResult<SendReceipt> {
        let result = self.upload_file(&s.submission, s.file, s.bytes, false)?;
        Ok(self.receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            result,
        ))
    }
    fn reconcile_file(
        &self,
        q: TextReceiptQuery<'_>,
        reference: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.reconcile_text(q, reference)
    }

    fn decode_received_voice(&self, silk: &[u8]) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        crate::voice_codec::decode(silk)
    }
    fn encode_voice(&self, wav: &[u8]) -> AppResult<werc_domain::voice::EncodedVoice> {
        if let Some((path, hash)) = &self.codec {
            crate::voice_codec::encode_at(path, hash, wav)
        } else {
            crate::voice_codec::encode(wav)
        }
    }
    fn submit_voice(&self, s: VoiceSubmission<'_>) -> AppResult<SendReceipt> {
        let result = self.upload_voice(&s.submission, s.voice, s.silk, false)?;
        Ok(self.receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            result,
        ))
    }
    fn reconcile_voice(
        &self,
        q: TextReceiptQuery<'_>,
        reference: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.reconcile_text(q, reference)
    }
    fn poll_receive(&self, account: &str) -> AppResult<ReceiveBatch> {
        if account != self.account_id {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        self.poll_native_messages()
    }
    fn acknowledge_receive(&self, account: &str, epoch: &str, sequence: u64) -> AppResult<()> {
        if account != self.account_id || epoch != self.context.epoch || sequence == 0 {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        let mut packet = channel_binding(&channel, 7);
        packet.put64(1512, sequence);
        channel.exchange(packet)?;
        Ok(())
    }
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        let mut capabilities = crate::WindowsWechatProvider.capabilities();
        // Explicit experiment is not a production readiness claim.
        for c in &mut capabilities {
            if c.id == "messages.send_text" {
                c.reason = "原生收件人通道已装配；仅精确构建，各目标的实际送达独立核验".into();
            }
        }
        for capability in self.profile.capabilities() {
            if let Some(existing)=capabilities.iter_mut().find(|c|c.id==capability.id) {*existing=capability;} else {capabilities.push(capability);}
        }
        capabilities
    }
    fn prepared_recipients(&self, account: &str) -> AppResult<Vec<PreparedRecipient>> {
        self.recipient_list(account)
    }
    fn prepare_client_recipient(
        &self,
        account: &str,
        target: &str,
    ) -> AppResult<PreparedRecipient> {
        self.prepare_client_target(account, target)
    }
    fn validate_receipt_context(&self, account: &str, context: &SendContext) -> AppResult<()> {
        self.receipt_context(account, context)
    }
    fn resolve_send_context(&self, account: &str, target: &str) -> AppResult<SendContext> {
        self.recipient_context(account, target)
    }
    fn submit_moment_action(
        &self,
        s: TextSubmission<'_>,
        action: &MomentAction,
    ) -> AppResult<SendReceipt> {
        self.require_feature(NativeFeature::MomentsActions)?;
        if matches!(action.action, MomentActionKind::Reply) { self.require_feature(NativeFeature::MomentsReply)?; }
        self.dispatch_moment_action(s, action)
    }
    fn reconcile_moment_action(
        &self,
        q: TextReceiptQuery<'_>,
        _: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.query_moment_action(q)
    }
    fn submit_video_moment(&self, s: MomentVideoSubmission<'_>, visibility: MomentVisibility) -> AppResult<SendReceipt> {
        self.require_feature(NativeFeature::MomentsPublish)?;
        self.publish_video_moment(s, visibility)
    }
    fn submit_moment(
        &self,
        s: MomentImageSubmission<'_>,
        visibility: MomentVisibility,
    ) -> AppResult<SendReceipt> {
        self.require_feature(NativeFeature::MomentsPublish)?;
        self.publish_moment(s, visibility)
    }
    fn submit_private_image_moment(&self, s: MomentImageSubmission<'_>) -> AppResult<SendReceipt> {
        self.require_feature(NativeFeature::MomentsPublish)?;
        self.publish_private_image_moment(s)
    }
    fn submit_private_moment(&self, s: TextSubmission<'_>) -> AppResult<SendReceipt> {
        self.require_feature(NativeFeature::MomentsPublish)?;
        self.publish_private_moment(s)
    }
    fn reconcile_private_moment(
        &self,
        q: TextReceiptQuery<'_>,
        _: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.query_private_moment(q)
    }
    fn submit_quote(&self, s: QuoteSubmission<'_>) -> AppResult<SendReceipt> {
        self.require_feature(NativeFeature::Quotes)?;
        let (code, _) = self.dispatch_quote(&s.submission, s.source, false)?;
        Ok(self.receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            code,
        ))
    }
    fn submit_mentions(&self, s: MentionSubmission<'_>) -> AppResult<SendReceipt> {
        self.require_feature(NativeFeature::Mentions)?;
        let result = self.dispatch_mentions(&s.submission, s.members, false)?;
        Ok(self.receipt(
            s.submission.outbox_id,
            s.submission.attempt_id,
            s.submission.context_digest,
            result,
        ))
    }
    fn submit_text(&self, submission: TextSubmission<'_>) -> AppResult<SendReceipt> {
        if !valid_send_text(submission.text) {
            return Err(error("NATIVE_TEXT_INVALID"));
        }
        if submission.text.len() >= 1024 {
            let code = self.dispatch_long_text(&submission, &[], false)?;
            return Ok(self.receipt(
                submission.outbox_id,
                submission.attempt_id,
                submission.context_digest,
                code,
            ));
        }
        if *submission.context
            != self.resolve_send_context(&self.account_id, &submission.context.target_id)?
        {
            return Err(error("NATIVE_CONTEXT_CHANGED"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| error("NATIVE_CLOCK_INVALID"))?
            .as_secs();
        if submission.deadline_unix <= now as i64 {
            return Err(error("NATIVE_APPROVAL_EXPIRED"));
        }
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        channel.check_identity()?;
        let mut p = channel_binding(&channel, 3);
        p.text(TARGET, 128, &submission.context.target_id)?;
        p.text(ATTEMPT, 64, submission.attempt_id)?;
        p.text(DIGEST, 65, submission.request_digest)?;
        p.text(TEXT, 1024, submission.text)?;
        p.put64(
            40,
            self.send_deadline(submission.context, submission.deadline_unix, now)?,
        );
        let result = channel.exchange(p)?;
        Ok(self.receipt(
            submission.outbox_id,
            submission.attempt_id,
            submission.context_digest,
            result.u32(1492),
        ))
    }
    fn reconcile_text(
        &self,
        q: TextReceiptQuery<'_>,
        previous: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.receipt_context(&self.account_id, q.context)?;
        let mut channel = self
            .transport
            .lock()
            .map_err(|_| error("NATIVE_DISCONNECTED"))?;
        let mut p = channel_binding(&channel, 5);
        p.text(TARGET, 128, &q.context.target_id)?;
        p.text(ATTEMPT, 64, q.attempt_id)?;
        let result = channel.exchange(p)?;
        let mut receipt = self.receipt(
            q.outbox_id,
            q.attempt_id,
            q.context_digest,
            result.u32(1492),
        );
        // A nonzero task-local ID is durable query material, never a delivery assertion.
        if matches!(result.u32(1492), 2 | 5 | 7 | 9 | 11 | 13)
            && matches!(result.u32(1504), 144 | 145)
        {
            let value = result.string(TEXT, 1024)?;
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Fields {
                local: u32,
                client: u64,
                millis: u64,
                seconds: u32,
                #[serde(default = "default_identity_type")]
                message_type: u64,
            }
            fn default_identity_type() -> u64 {
                1
            }
            if !value.is_empty() {
                let fields: Fields =
                    serde_json::from_str(&value).map_err(|_| error("NATIVE_PROTOCOL_INVALID"))?;
                let _ = fields.client;
                if fields.local > 0
                    && fields.seconds > 0
                    && fields.millis / 1000 == fields.seconds as u64
                    && (NativeMessageIdentity {
                        local_id: fields.local,
                        timestamp: fields.seconds as i64,
                        message_type: fields.message_type,
                    })
                    .valid()
                {
                    receipt.provider_reference = Some(
                        NativeMessageIdentity {
                            local_id: fields.local,
                            timestamp: fields.seconds as i64,
                            message_type: fields.message_type,
                        }
                        .reference(),
                    );
                }
            }
        }
        if receipt
            .provider_reference
            .as_deref()
            .and_then(NativeMessageIdentity::parse)
            .is_none()
        {
            if let Some(identity) = previous.and_then(NativeMessageIdentity::parse) {
                receipt.provider_reference = Some(identity.reference());
            }
        }
        Ok(receipt)
    }
    fn protect_key(&self, account: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        crate::keys::protect(account, key)
    }
    fn unprotect_key(&self, account: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        crate::keys::unprotect(account, key)
    }
    fn inspect(&self, path: &str) -> AppResult<werc_domain::connection::ClientInstallation> {
        crate::discovery::inspect(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_packet_abi_and_nul_bounds() {
        let mut p = Packet::new(3);
        assert_eq!(p.0.len(), 1528);
        p.put64(1520, u64::MAX);
        assert_eq!(p.u64(1520).to_string(), "18446744073709551615");
        assert!(p.text(TEXT, 1024, &"x".repeat(1024)).is_err());
        assert!(p.text(TARGET, 128, "filehelper\0extra").is_err());
        p.text(TEXT, 1024, "自动化测试").unwrap();
        assert_eq!(p.string(TEXT, 1024).unwrap(), "自动化测试");
        assert_eq!(p.u32(1492), 0);
    }
}

#[cfg(test)]
pub(crate) fn owned_pending_download_provider() -> NativeFileHelperProvider {
    // Owned binary-IPC responder. It never opens a WeChat process or a network.
    let script = r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$begun=0;$released=0;while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if($r -eq 0){exit 0};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);if($op -eq 22){$begun=1};if($op -eq 23){[Array]::Copy([BitConverter]::GetBytes([uint32]1),0,$p,1492,4)};if($op -eq 25){$released+=1};if($op -eq 4){[Array]::Copy([BitConverter]::GetBytes([uint32]$begun),0,$p,1492,4);[Array]::Copy([BitConverter]::GetBytes([uint32]$released),0,$p,1496,4)};$o.Write($p,0,1528);$o.Flush()}"#;
    owned_script_provider(script)
}
#[cfg(test)]
fn owned_script_provider(script: &str) -> NativeFileHelperProvider {
    let mut child = Command::new(
        std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system directory"))
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args(["-NoProfile", "-NonInteractive", "-Command", script])
    .creation_flags(0x08000000)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let job = Job::attach(&child).unwrap();
    let input = child.stdin.take();
    let mut output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        loop {
            let mut p = Packet::new(0);
            let r = output.read_exact(p.0.as_mut()).map(|_| p);
            let failed = r.is_err();
            if tx.send(r).is_err() || failed {
                break;
            }
        }
    });
    let mut binding = Packet::new(0);
    binding.text(ACCOUNT, 128, "owned-fixture").unwrap();
    binding.put64(24, 1);
    NativeFileHelperProvider {
        profile: &crate::native_profiles::PROFILES[0],
        account_id: "owned".into(),
        context: SendContext {
            sender_id: "owned-fixture".into(),
            target_id: "filehelper".into(),
            target_display_name: "owned".into(),
            identity_verified: true,
            target_verified: true,
            epoch: "owned".into(),
            installation_sha256: "1".repeat(64),
            adapter_sha256: "2".repeat(64),
            execution_mode: SendExecutionMode::Synthetic,
        },
        transport: Mutex::new(Transport {
            child,
            input,
            response: rx,
            _job: job,
            _host_file: File::open(std::env::current_exe().unwrap()).unwrap(),
            binding,
            dead: false,
        }),
        codec: None,
        closing: std::sync::atomic::AtomicBool::new(false),
        download_guard: Mutex::new(()),
        recipients: Mutex::new(Vec::new()),
    }
}
#[cfg(test)]
mod download_concurrency_tests {
    use super::*;
    #[test]
    fn pending_download_yields_transport_and_cancel_releases_slot() {
        let p = owned_pending_download_provider();
        // Complete the responder startup before measuring download concurrency.
        // Parallel tests may otherwise spend this entire deadline launching PowerShell.
        {
            let mut c = p.transport.lock().unwrap();
            let request = c.binding.copy(4);
            assert_eq!(c.exchange(request).unwrap().u32(1492), 0);
        }
        let descriptor = werc_domain::received_media::ReceivedImageDescriptor {
            file_key: Zeroizing::new("aabb".into()),
            aes_key: Zeroizing::new("ab".repeat(16)),
            declared_size: 20,
        };
        std::thread::scope(|scope| {
            let job = scope.spawn(|| p.download_image_bytes(&p.context, &descriptor));
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                let started = {
                    let mut c = p.transport.lock().unwrap();
                    let request = c.binding.copy(4);
                    c.exchange(request).unwrap().u32(1492) == 1
                };
                if started {
                    break;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(
                p.resolve_send_context("owned", "filehelper").unwrap(),
                p.context
            );
            p.request_disconnect();
            assert_eq!(
                job.join().unwrap().err().unwrap().code,
                "NATIVE_DISCONNECTED"
            );
            let mut c = p.transport.lock().unwrap();
            let request = c.binding.copy(4);
            assert_eq!(
                c.exchange(request).unwrap().u32(1496),
                1,
                "release must run even after cancel"
            );
        });
    }
}

#[cfg(test)]
mod receive_voice_protocol_tests {
    use super::*;
    #[test]
    fn fragmented_binary_voice_is_bound_to_observation_and_rejects_malformed_responses() {
        for mode in 0..11 {
            let script=r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$mode=MODE;
function W32([int]$at,[uint32]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,4)}
function W64([int]$at,[uint64]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,8)}
while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if($r -eq 0){exit 0};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);
if($op -eq 4){[Array]::Clear($p,465,256);[Array]::Copy([Text.Encoding]::ASCII.GetBytes('owned-fixture'),0,$p,465,13);[Array]::Copy([Text.Encoding]::ASCII.GetBytes('filehelper'),0,$p,593,10);[Array]::Copy([Text.Encoding]::ASCII.GetBytes('filehelper'),0,$p,208,10);W32 1496 1;W32 1492 34;W64 40 3;W64 1512 1;W64 1520 987;W32 1504 2057;if($mode -eq 6){W32 1504 262145};if($mode -eq 7){$p[208]=120};if($mode -eq 8){$p[465]=120};if($mode -eq 9){for($j=465;$j -lt 593;$j++){$p[$j]=120}};if($mode -eq 10){W32 1496 2}}
if($op -eq 21){W32 1496 3;W32 1492 34;W64 40 3;W64 1520 987;$p[465]=60;$p[466]=47;$p[467]=62}
if($op -eq 26){$at=[BitConverter]::ToUInt32($p,1500);$count=[Math]::Min(1024,2057-$at);W32 1496 $count;W32 1492 34;W64 40 2057;W64 1520 987;for($k=0;$k -lt $count;$k++){$p[465+$k]=[byte](($at+$k)%251)};
if($mode -eq 1){W32 1496 1025};if($mode -eq 2){W64 1512 2};if($mode -eq 3){W64 1520 988};if($mode -eq 4){W64 40 2058};if($mode -eq 5){W32 1492 3}}
$o.Write($p,0,1528);$o.Flush()}"#.replace("MODE",&mode.to_string());
            let p = owned_script_provider(&script);
            let result = p.poll_native_messages();
            if mode == 0 {
                let batch = result.unwrap();
                assert_eq!(batch.message.unwrap().native_message_id, "987");
                assert_eq!(
                    &*batch.inline_voice.unwrap(),
                    &(0..2057).map(|n| (n % 251) as u8).collect::<Vec<_>>()
                );
            } else {
                assert_eq!(result.err().unwrap().code, "NATIVE_PROTOCOL_INVALID");
            }
        }
    }
}

#[cfg(test)]
mod original_image_protocol_tests {
    use super::*;
    #[test]
    fn original_operation_exact_size_chunks_and_release() {
        for mismatch in [false, true] {
            let script=r#"$ErrorActionPreference='Stop';$i=[Console]::OpenStandardInput();$o=[Console]::OpenStandardOutput();$p=New-Object byte[] 1528;$released=0;
function W32([int]$at,[uint32]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,4)}
function W64([int]$at,[uint64]$v){[Array]::Copy([BitConverter]::GetBytes($v),0,$p,$at,8)}
while($true){$n=0;while($n -lt 1528){$r=$i.Read($p,$n,1528-$n);if($r -eq 0){exit 0};$n+=$r};$op=[BitConverter]::ToUInt32($p,8);
if($op -eq 49){if([BitConverter]::ToUInt32($p,1500) -ne 1 -or [BitConverter]::ToUInt32($p,4) -ne PROTOCOL){W32 12 13}}
elseif($op -eq 23){W32 1492 2;W64 1520 SIZE}
elseif($op -eq 24){W32 1492 2;W32 1496 3;$p[465]=97;$p[466]=98;$p[467]=99}
elseif($op -eq 25){$released++}
elseif($op -eq 4){W32 1496 $released}
elseif($op -eq 1){}
else{W32 12 13}
$o.Write($p,0,1528);$o.Flush()}"#.replace("SIZE",if mismatch {"4"}else{"3"}).replace("PROTOCOL",&NATIVE_PROTOCOL_VERSION.to_string());
            let p = owned_script_provider(&script);
            let descriptor = werc_domain::received_media::ReceivedImageDescriptor {
                file_key: Zeroizing::new("aabb".into()),
                aes_key: Zeroizing::new("ab".repeat(16)),
                declared_size: 3,
            };
            let result = p.download_original_image(&p.context, &descriptor);
            if mismatch {
                assert_eq!(result.err().unwrap().code, "NATIVE_PROTOCOL_INVALID");
            } else {
                assert_eq!(&*result.unwrap(), b"abc");
            }
            let mut c = p.transport.lock().unwrap();
            let request = c.binding.copy(4);
            assert_eq!(c.exchange(request).unwrap().u32(1496), 1);
        }
    }
}
