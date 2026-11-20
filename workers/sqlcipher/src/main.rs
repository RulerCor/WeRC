mod special_records;
#[path = "../../../crates/domain/src/special_records.rs"]
mod special_records_protocol;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::json;
use std::{
    io::{Read, Write},
    path::{Component, Path},
};
use zeroize::Zeroizing;
mod calendar;
mod contacts;
mod favorites;
#[path = "../../../crates/domain/src/favorites.rs"]
mod favorites_protocol;
#[path = "../../../crates/domain/src/group.rs"]
mod group_protocol;
mod groups;
mod live;
#[path = "../../../crates/domain/src/live.rs"]
mod live_protocol;
mod moments;
#[path = "../../../crates/domain/src/moments.rs"]
mod moments_protocol;
mod search;
mod sessions;
mod text;
mod voice;
mod window;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    action: String,
    records: Option<special_records_protocol::RecordSelection>,
    favorites: Option<favorites_protocol::FavoriteSelection>,
    search: Option<live_protocol::LiveSearchSelection>,
    calendar: Option<live_protocol::LiveCalendarSelection>,
    group: Option<group_protocol::LiveGroupSelection>,
    conversation: Option<live_protocol::LiveConversationSelection>,
    path: Option<String>,
    key_hex: Option<String>,
    limit: Option<usize>,
    before: Option<live_protocol::LiveAnchor>,
    voice: Option<live_protocol::LiveVoiceAnchor>,
    contact_after: Option<live_protocol::LiveContactAnchor>,
    contact_query: Option<String>,
    session_after: Option<live_protocol::LiveSessionAnchor>,
    session_query: Option<String>,
    moment_after: Option<moments_protocol::LiveMomentAnchor>,
    moment_media: Option<moments_protocol::MomentMediaSelection>,
}
impl Drop for Request {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.contact_query.zeroize();
        self.session_query.zeroize();
        if let Some(key) = self.key_hex.as_mut() {
            key.zeroize();
        }
    }
}
fn run(mut request: Request) -> Result<serde_json::Value, &'static str> {
    if request.action == "read_moment_media" {
        if request.records.is_some()
            || request.favorites.is_some()
            || request.search.is_some()
            || request.calendar.is_some()
            || request.group.is_some()
            || request.conversation.is_some()
            || request.limit.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.contact_query.is_some()
            || request.session_after.is_some()
            || request.session_query.is_some()
            || request.moment_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(moments::media(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request
                .moment_media
                .as_ref()
                .ok_or("MOMENT_MEDIA_INVALID")?,
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.moment_media.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_special_records" {
        if request.favorites.is_some()
            || request.search.is_some()
            || request.calendar.is_some()
            || request.group.is_some()
            || request.conversation.is_some()
            || request.limit.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.contact_query.is_some()
            || request.session_after.is_some()
            || request.session_query.is_some()
            || request.moment_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(special_records::read(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.records.as_ref().ok_or("RECORD_SELECTION_INVALID")?,
        )?)
        .map_err(|_| "RECORD_READ_FAILED");
    }
    if request.records.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_favorites" {
        if request.search.is_some()
            || request.calendar.is_some()
            || request.group.is_some()
            || request.conversation.is_some()
            || request.limit.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.contact_query.is_some()
            || request.session_after.is_some()
            || request.session_query.is_some()
            || request.moment_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(favorites::read(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request
                .favorites
                .as_ref()
                .ok_or("FAVORITE_SELECTION_INVALID")?,
        )?)
        .map_err(|_| "FAVORITE_READ_FAILED");
    }
    if request.favorites.is_some() {
        return Err("INVALID_REQUEST");
    }

    if request.action == "read_calendar" {
        if request.search.is_some()
            || request.group.is_some()
            || request.conversation.is_some()
            || request.limit.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.contact_query.is_some()
            || request.session_after.is_some()
            || request.session_query.is_some()
            || request.moment_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(calendar::read(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.calendar.as_ref().ok_or("LIVE_CALENDAR_INVALID")?,
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.calendar.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "search_messages" {
        if request.group.is_some()
            || request.conversation.is_some()
            || request.limit.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.contact_query.is_some()
            || request.session_after.is_some()
            || request.session_query.is_some()
            || request.moment_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(search::read(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.search.as_ref().ok_or("LIVE_QUERY_INVALID")?,
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.search.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.session_query.is_some() && request.action != "read_sessions" {
        return Err("INVALID_REQUEST");
    }
    if request.contact_query.is_some() && request.action != "read_contacts" {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_group_members" {
        if request.conversation.is_some()
            || request.limit.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.session_after.is_some()
            || request.moment_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(groups::read(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request
                .group
                .as_ref()
                .ok_or("LIVE_GROUP_SELECTION_INVALID")?,
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.group.is_some() {
        return Err("INVALID_REQUEST");
    }

    if request.action == "read_moments" {
        if request.conversation.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.session_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(moments::read_batch(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.moment_after.as_ref(),
            request
                .limit
                .unwrap_or(moments_protocol::LIVE_MOMENT_MAX_ROWS),
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.moment_after.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_conversation_window" {
        if request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
            || request.session_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        let selection = request
            .conversation
            .as_ref()
            .ok_or("LIVE_CONVERSATION_INVALID")?;
        if !selection.valid() {
            return Err("LIVE_CONVERSATION_INVALID");
        }
        return serde_json::to_value(window::read_selected(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.limit.ok_or("LIVE_WINDOW_LIMIT")?,
            Some(selection),
            None,
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.conversation.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_sessions" {
        if request.limit.is_some()
            || request.before.is_some()
            || request.voice.is_some()
            || request.contact_after.is_some()
        {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(sessions::read_filtered(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.session_after.as_ref(),
            request.session_query.as_deref(),
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.session_after.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_contacts" {
        if request.limit.is_some() || request.before.is_some() || request.voice.is_some() {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(contacts::read_filtered(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.contact_after.as_ref(),
            request.contact_query.as_deref(),
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.contact_after.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_filehelper_voice" {
        if request.limit.is_some() || request.before.is_some() {
            return Err("INVALID_REQUEST");
        }
        return serde_json::to_value(voice::read(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.voice.as_ref().ok_or("LIVE_VOICE_ANCHOR_INVALID")?,
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.voice.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "read_filehelper_window" {
        return serde_json::to_value(window::read(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            request.limit.ok_or("LIVE_WINDOW_LIMIT")?,
            request.before.as_ref(),
        )?)
        .map_err(|_| "LIVE_READ_FAILED");
    }
    if request.limit.is_some() || request.before.is_some() {
        return Err("INVALID_REQUEST");
    }
    if request.action == "probe_live" {
        return live::probe(
            Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?),
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
        );
    }
    if request.action == "self_check" {
        if request.path.is_some() || request.key_hex.is_some() {
            return Err("INVALID_REQUEST");
        }
        let c = Connection::open_in_memory().map_err(|_| "ENGINE_UNAVAILABLE")?;
        let cipher: String = c
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .map_err(|_| "SQLCIPHER_REQUIRED")?;
        if cipher.is_empty() {
            return Err("SQLCIPHER_REQUIRED");
        }
        return Ok(
            json!({"engine":"sqlcipher","cipher_version":cipher,"isolated_process":true,"wechat_verified":false}),
        );
    }
    if !matches!(request.action.as_str(), "create_fixture" | "read_fixture") {
        return Err("UNKNOWN_ACTION");
    }
    let path = Path::new(request.path.as_deref().ok_or("PATH_REQUIRED")?);
    if !path.is_absolute()
        || path.components().any(|c| matches!(c, Component::ParentDir))
        || !path
            .file_name()
            .is_some_and(|s| s.to_string_lossy().ends_with(".werc-fixture.sqlite3"))
    {
        return Err("SYNTHETIC_FIXTURE_ONLY");
    }
    for ancestor in path.ancestors().skip(1) {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| "UNSAFE_PATH")?;
        if metadata.file_type().is_symlink() {
            return Err("UNSAFE_PATH");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("UNSAFE_PATH");
            }
        }
    }
    let key = Zeroizing::new(request.key_hex.take().ok_or("KEY_REQUIRED")?);
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("KEY_FORMAT_INVALID");
    }
    let create = request.action == "create_fixture";
    if create {
        if std::env::var("WERC_ALLOW_SYNTHETIC_FIXTURE").as_deref() != Ok("1") {
            return Err("FIXTURE_CREATION_DISABLED");
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| "DESTINATION_EXISTS_OR_DENIED")?;
    } else {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| "SOURCE_NOT_FOUND")?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > 16 * 1024 * 1024
        {
            return Err("UNSAFE_PATH");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("UNSAFE_PATH");
            }
        }
    }
    let flags = if create {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let c = Connection::open_with_flags(path, flags).map_err(|_| "DATABASE_OPEN_FAILED")?;
    let sql = Zeroizing::new(format!(
        "PRAGMA key = \"x'{}'\"; PRAGMA cipher_memory_security=ON; PRAGMA temp_store=MEMORY; PRAGMA trusted_schema=OFF;",
        key.as_str()
    ));
    c.execute_batch(sql.as_str())
        .map_err(|_| "DATABASE_KEY_REJECTED")?;
    if create {
        c.execute_batch("CREATE TABLE werc_fixture_marker(version INTEGER NOT NULL); INSERT INTO werc_fixture_marker VALUES(1); CREATE TABLE fixture_messages(id INTEGER PRIMARY KEY,body TEXT NOT NULL); INSERT INTO fixture_messages VALUES(1,'synthetic-one'),(2,'synthetic-two');").map_err(|_| "FIXTURE_CREATION_FAILED")?;
    }
    c.execute_batch("PRAGMA query_only=ON;")
        .map_err(|_| "READ_ONLY_REQUIRED")?;
    let version: i64 = c
        .query_row("SELECT version FROM werc_fixture_marker", [], |r| r.get(0))
        .map_err(|_| "KEY_OR_FIXTURE_INVALID")?;
    if version != 1 {
        return Err("FIXTURE_SCHEMA_UNSUPPORTED");
    }
    let mut statement = c
        .prepare("SELECT id,body FROM fixture_messages ORDER BY id LIMIT 100")
        .map_err(|_| "FIXTURE_SCHEMA_UNSUPPORTED")?;
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|_| "DATABASE_READ_FAILED")?;
    let mut messages = Vec::new();
    for row in rows {
        let (id, body) = row.map_err(|_| "DATABASE_READ_FAILED")?;
        if body.len() > 4096 {
            return Err("ROW_LIMIT_EXCEEDED");
        }
        messages.push(json!({"id":id,"body":body}));
    }
    Ok(json!({"fixture_schema":"werc_fixture_v1","data":messages,"wechat_verified":false}))
}
fn main() {
    // 一次性子进程协议；密钥只通过继承的匿名stdin管道进入，不接受argv。
    if std::env::args_os().len() != 1 {
        std::process::exit(2);
    }
    let mut input = Zeroizing::new(Vec::new());
    let reply = if std::io::stdin()
        .take(65537)
        .read_to_end(&mut input)
        .is_err()
        || input.len() > 65536
    {
        json!({"ok":false,"error":"REQUEST_LIMIT_EXCEEDED"})
    } else {
        match serde_json::from_slice::<Request>(&input)
            .map_err(|_| "INVALID_REQUEST")
            .and_then(run)
        {
            Ok(value) => json!({"ok":true,"data":value}),
            Err(code) => json!({"ok":false,"error":code}),
        }
    };
    if let Ok(output) = serde_json::to_vec(&reply) {
        let _ = std::io::stdout().write_all(&output);
    }
}
