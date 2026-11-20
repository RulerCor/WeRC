//! Bounded read-only transactions via a pinned SQLCipher child and Job.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use werc_domain::{AppError, AppResult};
use zeroize::Zeroizing;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveProbe {
    pub schema_objects: u64,
    pub database_pages: u64,
    pub cipher_version: String,
    pub sqlite_version: String,
    pub read_only: bool,
    pub readonly_shm: bool,
    pub immutable: bool,
    pub rows_exported: u64,
    pub full_g1a_verified: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply<T> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}
fn error(code: &str) -> AppError {
    let message = match code {
        "LIVE_CONVERSATION_NOT_IN_SOURCE" => "所选消息分片未保存此会话，请选择其他来源",
        "LIVE_IDENTITY_SCHEMA_UNSUPPORTED" => "当前消息来源的会话身份结构尚未适配",
        "LIVE_MOMENT_ANCHOR_CHANGED" => "朋友圈边界已变化，请重新读取",
        "LIVE_MOMENT_SCHEMA_UNSUPPORTED" => "当前微信的朋友圈缓存结构尚未适配",
        "LIVE_SESSION_ANCHOR_CHANGED" => "会话顺序已变化，请重新读取会话列表",
        "LIVE_GROUP_ANCHOR_CHANGED" => "群成员边界已变化，请重新读取",
        "LIVE_GROUP_SCHEMA_UNSUPPORTED" => "当前微信的群成员缓存结构尚未适配",
        "LIVE_GROUP_IDENTITY_UNVERIFIED" => "本地群身份映射无法核对",
        "LIVE_GROUP_NOT_IN_CACHE" => "本地缓存没有此群资料",
        "LIVE_CONTACT_ANCHOR_CHANGED" => "联系人边界已变化，请重新读取联系人列表",
        "LIVE_SESSION_SCHEMA_UNSUPPORTED" => "当前微信的会话库结构尚未适配",
        "LIVE_CONTACT_SCHEMA_UNSUPPORTED" => "当前微信的联系人库结构尚未适配",
        "QUERY_CANCELED" => "读取已取消",
        "LIVE_WORKER_TIMEOUT" | "LIVE_READ_TIMEOUT" => {
            "本次来源查询超时，请缩小日期范围或增加筛选条件后重试"
        }
        "LIVE_QUERY_INVALID" => "查询条件无效，请检查日期范围、账号标识与搜索文字长度",
        "LIVE_QUERY_UNAVAILABLE" => "当前读取组件尚未提供来源搜索，请使用与主程序匹配的组件",
        _ => "在线只读引擎未完成读取；未提交微信操作",
    };
    AppError::new(code, message)
}
fn pin(exe: &Path, expected: &str) -> AppResult<File> {
    if !exe.is_absolute()
        || expected.len() != 64
        || exe.file_name().and_then(|s| s.to_str()) != Some("werc-sqlcipher-worker.exe")
    {
        return Err(error("LIVE_COMPONENT_INVALID"));
    }
    for p in exe.ancestors() {
        if std::fs::symlink_metadata(p)
            .map_err(|_| error("LIVE_COMPONENT_INVALID"))?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err(error("LIVE_COMPONENT_INVALID"));
        }
    }
    let mut f = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(exe)
        .map_err(|_| error("LIVE_COMPONENT_INVALID"))?;
    if f.metadata()
        .map_err(|_| error("LIVE_COMPONENT_INVALID"))?
        .len()
        > 64 * 1024 * 1024
    {
        return Err(error("LIVE_COMPONENT_INVALID"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = f
            .read(&mut buffer)
            .map_err(|_| error("LIVE_COMPONENT_INVALID"))?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    if hex::encode(digest.finalize()) != expected {
        return Err(error("LIVE_COMPONENT_INVALID"));
    }
    Ok(f)
}
pub fn verify_component(exe: &Path, expected: &str) -> AppResult<()> {
    drop(pin(exe, expected)?);
    Ok(())
}
pub fn filehelper_voice_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    anchor: &werc_domain::live::LiveVoiceAnchor,
    canceled: &AtomicBool,
) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
    if anchor.server_key().is_none() {
        return Err(error("LIVE_VOICE_ANCHOR_INVALID"));
    }
    let payload: werc_domain::live::LiveVoicePayload = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            calendar: None,
            search: None,
            conversation: None,
            action: "read_filehelper_voice",
            limit: None,
            before: None,
            voice: Some(anchor),
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: 524288 + 1024,
            canceled,
        },
    )?;
    validate_voice_payload(payload)
}
fn validate_voice_payload(
    payload: werc_domain::live::LiveVoicePayload,
) -> AppResult<Option<Zeroizing<Vec<u8>>>> {
    if payload.schema_version != 1 || !payload.read_only {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    let Some(encoded) = payload.silk_hex.as_ref() else {
        return Ok(None);
    };
    if encoded.is_empty() || encoded.len() > 524288 || encoded.len() % 2 != 0 {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    let bytes = Zeroizing::new(hex::decode(encoded).map_err(|_| error("LIVE_PROTOCOL_INVALID"))?);
    werc_domain::voice::silk_duration(&bytes).map_err(|_| error("LIVE_PROTOCOL_INVALID"))?;
    Ok(Some(bytes))
}

struct InvokeOptions<'a> {
    records: Option<&'a werc_domain::special_records::RecordSelection>,
    favorites: Option<&'a werc_domain::favorites::FavoriteSelection>,
    group: Option<&'a werc_domain::group::LiveGroupSelection>,
    conversation: Option<&'a werc_domain::live::LiveConversationSelection>,
    search: Option<&'a werc_domain::live::LiveSearchSelection>,
    calendar: Option<&'a werc_domain::live::LiveCalendarSelection>,
    action: &'static str,
    limit: Option<usize>,
    before: Option<&'a werc_domain::live::LiveAnchor>,
    voice: Option<&'a werc_domain::live::LiveVoiceAnchor>,
    contact_after: Option<&'a werc_domain::live::LiveContactAnchor>,
    contact_query: Option<&'a str>,
    session_after: Option<&'a werc_domain::live::LiveSessionAnchor>,
    session_query: Option<&'a str>,
    moment_media: Option<&'a werc_domain::moments::MomentMediaSelection>,
    moment_after: Option<&'a werc_domain::moments::LiveMomentAnchor>,
    response_limit: usize,
    canceled: &'a AtomicBool,
}
fn invoke<T: serde::de::DeserializeOwned>(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    options: InvokeOptions<'_>,
) -> AppResult<T> {
    let InvokeOptions {
        records,
        favorites,
        group,
        action,
        conversation,
        search,
        calendar,
        limit,
        before,
        voice,
        contact_after,
        contact_query,
        session_after,
        session_query,
        moment_media,
        moment_after,
        response_limit,
        canceled,
    } = options;
    if canceled.load(Ordering::SeqCst) {
        return Err(error("QUERY_CANCELED"));
    }
    if key.len() != 32 {
        return Err(error("KEY_FORMAT_INVALID"));
    }
    let _component = pin(exe, expected)?;
    #[derive(Serialize)]
    struct Request<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        records: Option<&'a werc_domain::special_records::RecordSelection>,
        #[serde(skip_serializing_if = "Option::is_none")]
        favorites: Option<&'a werc_domain::favorites::FavoriteSelection>,
        group: Option<&'a werc_domain::group::LiveGroupSelection>,
        action: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation: Option<&'a werc_domain::live::LiveConversationSelection>,
        #[serde(skip_serializing_if = "Option::is_none")]
        search: Option<&'a werc_domain::live::LiveSearchSelection>,
        #[serde(skip_serializing_if = "Option::is_none")]
        calendar: Option<&'a werc_domain::live::LiveCalendarSelection>,
        path: &'a Path,
        key_hex: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        before: Option<&'a werc_domain::live::LiveAnchor>,
        #[serde(skip_serializing_if = "Option::is_none")]
        voice: Option<&'a werc_domain::live::LiveVoiceAnchor>,
        #[serde(skip_serializing_if = "Option::is_none")]
        contact_after: Option<&'a werc_domain::live::LiveContactAnchor>,
        #[serde(skip_serializing_if = "Option::is_none")]
        contact_query: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_after: Option<&'a werc_domain::live::LiveSessionAnchor>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_query: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        moment_media: Option<&'a werc_domain::moments::MomentMediaSelection>,
        moment_after: Option<&'a werc_domain::moments::LiveMomentAnchor>,
    }
    let key_hex = Zeroizing::new(hex::encode(key));
    let input = Zeroizing::new(
        serde_json::to_vec(&Request {
            records,
            favorites,
            group,
            action,
            conversation,
            search,
            calendar,
            path,
            key_hex: &key_hex,
            limit,
            before,
            voice,
            contact_after,
            contact_query,
            session_after,
            session_query,
            moment_media,
            moment_after,
        })
        .map_err(|_| error("LIVE_PROTOCOL_INVALID"))?,
    );
    if input.len() > 65536 {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    let mut child = Command::new(exe)
        .current_dir(
            exe.parent()
                .ok_or_else(|| error("LIVE_COMPONENT_INVALID"))?,
        )
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error("LIVE_WORKER_UNAVAILABLE"))?;
    let job = match crate::native_messages::Job::attach(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let writer = std::thread::spawn(move || {
        let r = stdin.write_all(&input);
        drop(stdin);
        r
    });
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        let result = stdout
            .take(response_limit as u64 + 1)
            .read_to_end(&mut bytes);
        let _ = tx.send((result, bytes));
    });
    let started = Instant::now();
    let received = loop {
        if canceled.load(Ordering::SeqCst) || started.elapsed() >= Duration::from_secs(10) {
            break Err(mpsc::RecvTimeoutError::Timeout);
        }
        match rx.recv_timeout(Duration::from_millis(25)) {
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            result => break result,
        }
    };
    if received.is_err()
        || received
            .as_ref()
            .is_ok_and(|(result, bytes)| result.is_err() || bytes.len() > response_limit)
    {
        let _ = child.kill();
    }
    // EOF is required; a worker that writes a response but stays alive is killed.
    let exited = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Err(_) => break false,
            _ => (),
        }
        if canceled.load(Ordering::SeqCst) || started.elapsed() >= Duration::from_secs(10) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !exited {
        let _ = child.kill();
    }
    let _ = child.wait();
    drop(job);
    let wrote = matches!(writer.join(), Ok(Ok(())));
    let read_joined = reader.join().is_ok();
    if canceled.load(Ordering::SeqCst) {
        return Err(error("QUERY_CANCELED"));
    }
    if !wrote || !read_joined {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    let (read, bytes) = received.map_err(|_| error("LIVE_WORKER_TIMEOUT"))?;
    if read.is_err() || bytes.len() > response_limit || !exited {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    let reply: Reply<T> =
        serde_json::from_slice(&bytes).map_err(|_| error("LIVE_PROTOCOL_INVALID"))?;
    if !reply.ok {
        let code = reply
            .error
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
            })
            .ok_or_else(|| error("LIVE_PROTOCOL_INVALID"))?;
        return Err(error(&code));
    }
    if reply.error.is_some() {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    reply.data.ok_or_else(|| error("LIVE_PROTOCOL_INVALID"))
}
pub fn probe(exe: &Path, expected: &str, path: &Path, key: &[u8]) -> AppResult<LiveProbe> {
    probe_cancellable(exe, expected, path, key, &AtomicBool::new(false))
}
pub fn probe_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    canceled: &AtomicBool,
) -> AppResult<LiveProbe> {
    let value: LiveProbe = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            calendar: None,
            search: None,
            conversation: None,
            action: "probe_live",
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: 65536,
            canceled,
        },
    )?;
    if !value.read_only
        || !value.readonly_shm
        || value.immutable
        || value.rows_exported != 0
        || value.full_g1a_verified
        || value.schema_objects > 100000
        || value.database_pages > 262144
        || value.cipher_version.len() > 64
        || value.sqlite_version.len() > 32
    {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    Ok(value)
}

pub fn filehelper_window(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    limit: usize,
) -> AppResult<werc_domain::live::LiveWindow> {
    filehelper_window_cancellable(exe, expected, path, key, limit, &AtomicBool::new(false))
}
pub fn filehelper_window_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    limit: usize,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveWindow> {
    filehelper_window_before_cancellable(exe, expected, path, key, limit, None, canceled)
}
pub fn filehelper_window_before_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    limit: usize,
    before: Option<&werc_domain::live::LiveAnchor>,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveWindow> {
    conversation_window_cancellable(
        exe,
        expected,
        path,
        key,
        limit,
        &werc_domain::live::LiveConversationSelection {
            username: "filehelper".into(),
            before: before.cloned(),
            include_anchor: false,
            newer_context: false,
            query: None,
        },
        canceled,
    )
}
pub fn calendar_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    selection: &werc_domain::live::LiveCalendarSelection,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveCalendarWindow> {
    if !selection.valid() {
        return Err(error("LIVE_CALENDAR_INVALID"));
    }
    let result: werc_domain::live::LiveCalendarWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            conversation: None,
            search: None,
            calendar: Some(selection),
            action: "read_calendar",
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: 8192,
            canceled,
        },
    )?;
    if !result.valid(selection) {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    Ok(result)
}
pub fn search_messages_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    selection: &werc_domain::live::LiveSearchSelection,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveSearchWindow> {
    if !selection.valid() {
        return Err(error("LIVE_QUERY_INVALID"));
    }
    let window: werc_domain::live::LiveSearchWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            conversation: None,
            calendar: None,
            search: Some(selection),
            action: "search_messages",
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: werc_domain::live::LIVE_WINDOW_MAX_BYTES + 1024,
            canceled,
        },
    )?;
    if !window.valid(selection) {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    Ok(window)
}
pub fn conversation_window_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    limit: usize,
    selection: &werc_domain::live::LiveConversationSelection,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveWindow> {
    use werc_domain::live::*;
    let before = selection.before.as_ref();
    let direct_filehelper = selection.username == "filehelper"
        && selection.query.is_none()
        && !selection.include_anchor
        && !selection.newer_context;
    if !selection.valid() {
        return Err(error("LIVE_CONVERSATION_INVALID"));
    }
    let boundary = before
        .map(|a| a.key().ok_or_else(|| error("LIVE_ANCHOR_INVALID")))
        .transpose()?;
    if !(1..=LIVE_WINDOW_MAX_ROWS).contains(&limit) {
        return Err(error("LIVE_WINDOW_LIMIT"));
    }
    let window: LiveWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            calendar: None,
            search: None,
            conversation: (!direct_filehelper).then_some(selection),
            action: if direct_filehelper {
                "read_filehelper_window"
            } else {
                "read_conversation_window"
            },
            limit: Some(limit),
            before: direct_filehelper.then_some(before).flatten(),
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: LIVE_WINDOW_MAX_BYTES + 1024,
            canceled,
        },
    )?;
    if window.schema_version != 1
        || !window.read_only
        || !window.readonly_shm
        || window.read_mode != "bounded_transaction_window"
        || window.rows.len()
            > if selection.newer_context {
                limit.min(30)
            } else {
                limit
            }
        || (window.has_older && window.rows.len() != limit)
    {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    let mut previous = None;
    for row in &window.rows {
        let id = row
            .local_id
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0 && n.to_string() == row.local_id)
            .ok_or_else(|| error("LIVE_PROTOCOL_INVALID"))?;
        if row.timestamp < 0
            || selection.query.as_ref().is_some_and(|q| !q.matches(row))
            || boundary.is_some_and(|bound| {
                if selection.newer_context {
                    (row.timestamp, id) <= bound
                } else if selection.include_anchor {
                    (row.timestamp, id) > bound
                } else {
                    (row.timestamp, id) >= bound
                }
            })
            || row.message_type.parse::<i64>().is_err()
            || row.at_user_list.as_ref().is_some_and(|users| users.len()>256 || users.iter().any(|s| !werc_domain::live::valid_live_username(s)))
            || row
                .sender_username
                .as_ref()
                .is_some_and(|s| !werc_domain::live::valid_live_username(s))
            || row
                .server_id
                .as_ref()
                .is_some_and(|s| s.parse::<i64>().is_err())
            || previous.is_some_and(|p| (row.timestamp, id) >= p)
            || row
                .text
                .as_ref()
                .is_some_and(|t| t.len() > 16384 || t.contains('\0'))
            || row.metadata.as_ref().is_some_and(|m| {
                m.len() > 16384
                    || m.contains('\0')
                    || row.message_type.parse::<i64>().map_or(true, |t| {
                        !matches!(t & 0xffffffff, 3 | 37 | 42 | 43 | 47 | 48 | 49 | 50 | 10000 | 10002)
                    })
            })
            || !matches!(
                (
                    row.content_status.as_str(),
                    row.text.is_some(),
                    row.metadata.is_some()
                ),
                ("text" | "text_zstd", true, false)
                    | ("metadata" | "metadata_zstd", false, true)
                    | (
                        "unsupported_type"
                            | "content_missing"
                            | "content_invalid"
                            | "content_oversized"
                            | "compression_unsupported",
                        false,
                        false
                    )
            )
        {
            return Err(error("LIVE_PROTOCOL_INVALID"));
        }
        previous = Some((row.timestamp, id));
    }
    Ok(window)
}

pub fn group_members_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    selection: &werc_domain::group::LiveGroupSelection,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::group::LiveGroupWindow> {
    use werc_domain::group::*;
    if !selection.valid() {
        return Err(error("LIVE_GROUP_SELECTION_INVALID"));
    }
    let window: LiveGroupWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            action: "read_group_members",
            records: None,
            favorites: None,
            group: Some(selection),
            calendar: None,
            search: None,
            conversation: None,
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: LIVE_GROUP_MAX_BYTES + 1024,
            canceled,
        },
    )?;
    if !window.valid(selection) {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    Ok(window)
}

pub fn contacts_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    after: Option<&werc_domain::live::LiveContactAnchor>,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveContactWindow> {
    contacts_filtered_cancellable(exe, expected, path, key, after, None, canceled)
}

pub fn contacts_filtered_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    after: Option<&werc_domain::live::LiveContactAnchor>,
    search_query: Option<&str>,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveContactWindow> {
    use werc_domain::live::*;
    if search_query.is_some_and(|q| !valid_contact_query(q)) {
        return Err(error("LIVE_QUERY_INVALID"));
    }
    if after.is_some_and(|a| a.key().is_none()) {
        return Err(error("LIVE_CONTACT_ANCHOR_INVALID"));
    }
    let window: LiveContactWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            calendar: None,
            search: None,
            conversation: None,
            action: "read_contacts",
            limit: None,
            before: None,
            voice: None,
            contact_after: after,
            contact_query: search_query,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: LIVE_CONTACT_MAX_BYTES + 1024,
            canceled,
        },
    )?;
    if !window.valid(after)
        || search_query.is_some_and(|q| window.rows.iter().any(|row| !row.matches_query(q)))
    {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    Ok(window)
}

pub fn sessions_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    after: Option<&werc_domain::live::LiveSessionAnchor>,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveSessionWindow> {
    sessions_filtered_cancellable(exe, expected, path, key, after, None, canceled)
}

pub fn sessions_filtered_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    after: Option<&werc_domain::live::LiveSessionAnchor>,
    search_query: Option<&str>,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::live::LiveSessionWindow> {
    use werc_domain::live::*;
    if search_query.is_some_and(|q| !valid_session_query(q)) {
        return Err(error("LIVE_QUERY_INVALID"));
    }
    if after.is_some_and(|a| a.key().is_none()) {
        return Err(error("LIVE_SESSION_ANCHOR_INVALID"));
    }
    let window: LiveSessionWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            calendar: None,
            search: None,
            conversation: None,
            action: "read_sessions",
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: after,
            session_query: search_query,
            moment_media: None,
            moment_after: None,
            response_limit: LIVE_SESSION_MAX_BYTES + 1024,
            canceled,
        },
    )?;
    if !window.valid(after)
        || search_query.is_some_and(|q| window.rows.iter().any(|row| !row.matches_query(q)))
    {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    Ok(window)
}

pub fn moments_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    after: Option<&werc_domain::moments::LiveMomentAnchor>,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::moments::LiveMomentWindow> {
    moments_batch_cancellable(
        exe,
        expected,
        path,
        key,
        after,
        werc_domain::moments::LIVE_MOMENT_MAX_ROWS,
        canceled,
    )
}
pub fn moments_batch_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    after: Option<&werc_domain::moments::LiveMomentAnchor>,
    limit: usize,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::moments::LiveMomentWindow> {
    use werc_domain::moments::*;
    if !matches!(limit, LIVE_MOMENT_MAX_ROWS | LIVE_MOMENT_SCAN_ROWS) {
        return Err(error("LIVE_MOMENT_LIMIT"));
    }
    if after.is_some_and(|a| a.key().is_none()) {
        return Err(error("LIVE_MOMENT_ANCHOR_INVALID"));
    }
    let window: LiveMomentWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            calendar: None,
            search: None,
            conversation: None,
            action: "read_moments",
            limit: (limit != LIVE_MOMENT_MAX_ROWS).then_some(limit),
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: after,
            response_limit: if limit == LIVE_MOMENT_MAX_ROWS {
                LIVE_MOMENT_MAX_BYTES + 1024
            } else {
                LIVE_MOMENT_SCAN_BYTES + 1024
            },
            canceled,
        },
    )?;
    if !window.valid_with_limit(after, limit) {
        return Err(error("LIVE_PROTOCOL_INVALID"));
    }
    Ok(window)
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    use werc_domain::live::LiveVoicePayload;
    #[test]
    fn bounded_voice_reply_rejects_partial_packets_and_wrong_contract() {
        let valid = b"\x02#!SILK_V3\x01\x00\x03";
        let make = |data: Option<String>| LiveVoicePayload {
            schema_version: 1,
            read_only: true,
            silk_hex: data,
        };
        assert_eq!(
            &**validate_voice_payload(make(Some(hex::encode(valid))))
                .unwrap()
                .as_ref()
                .unwrap(),
            valid
        );
        assert!(validate_voice_payload(make(None)).unwrap().is_none());
        for value in [
            String::new(),
            "a".into(),
            "gg".into(),
            "00".repeat(262145),
            hex::encode(&valid[..12]),
        ] {
            assert!(validate_voice_payload(make(Some(value))).is_err());
        }
        let mut value = make(None);
        value.read_only = false;
        assert!(validate_voice_payload(value).is_err());
        let mut value = make(None);
        value.schema_version = 2;
        assert!(validate_voice_payload(value).is_err());
    }
}

pub fn favorites_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    selection: &werc_domain::favorites::FavoriteSelection,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::favorites::FavoriteWindow> {
    use werc_domain::favorites::*;
    if !selection.valid() {
        return Err(error("FAVORITE_SELECTION_INVALID"));
    }
    let result: FavoriteWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: Some(selection),
            group: None,
            conversation: None,
            search: None,
            calendar: None,
            action: "read_favorites",
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: MAX_BYTES + 1024,
            canceled,
        },
    )?;
    if !result.valid(selection) {
        return Err(error("FAVORITE_PROTOCOL_INVALID"));
    }
    Ok(result)
}

pub fn special_records_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    selection: &werc_domain::special_records::RecordSelection,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::special_records::RecordWindow> {
    use werc_domain::special_records::*;
    if !selection.valid() {
        return Err(error("RECORD_SELECTION_INVALID"));
    }
    let result: RecordWindow = invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: Some(selection),
            favorites: None,
            group: None,
            conversation: None,
            search: None,
            calendar: None,
            action: "read_special_records",
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_media: None,
            moment_after: None,
            response_limit: MAX_BYTES + 1024,
            canceled,
        },
    )?;
    if !result.valid(selection) {
        return Err(error("RECORD_PROTOCOL_INVALID"));
    }
    Ok(result)
}

pub fn moment_media_cancellable(
    exe: &Path,
    expected: &str,
    path: &Path,
    key: &[u8],
    selection: &werc_domain::moments::MomentMediaSelection,
    canceled: &AtomicBool,
) -> AppResult<werc_domain::moments::MomentMediaDescriptor> {
    if !selection.valid() {
        return Err(error("MOMENT_MEDIA_INVALID"));
    }
    invoke(
        exe,
        expected,
        path,
        key,
        InvokeOptions {
            records: None,
            favorites: None,
            group: None,
            conversation: None,
            search: None,
            calendar: None,
            action: "read_moment_media",
            limit: None,
            before: None,
            voice: None,
            contact_after: None,
            contact_query: None,
            session_after: None,
            session_query: None,
            moment_after: None,
            moment_media: Some(selection),
            response_limit: werc_domain::moments::LIVE_MOMENT_MAX_BYTES + 16384,
            canceled,
        },
    )
}
