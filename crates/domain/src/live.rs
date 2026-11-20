//! Shared bounded live-window wire contract. No public cursor or account binding.
use serde::{Deserialize, Serialize};
#[path = "live_query.rs"]
mod query_body;
#[path = "live_search.rs"]
mod search_protocol;
pub use search_protocol::*;
#[path = "live_calendar.rs"]
mod calendar_protocol;
pub use calendar_protocol::*;

#[path = "live_mentions.rs"]
mod mention_source;
pub use mention_source::parse_mention_source;

pub const LIVE_WINDOW_MAX_ROWS: usize = 2048;
pub const LIVE_WINDOW_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const FILEHELPER_TABLE: &str = "Msg_9e20f478899dc29eb19741386f9343c8";

pub const LIVE_CONTACT_MAX_ROWS: usize = 200;
pub const LIVE_CONTACT_MAX_BYTES: usize = 1024 * 1024;

/// Account shape for local browsing/export only, not native send capability.
pub fn is_group_account(username: &str) -> bool {
    username.ends_with("@chatroom") || username.ends_with("@im.chatroom")
}
pub fn is_private_account(username: &str) -> bool {
    !is_group_account(username) && !username.starts_with("gh_")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveContactContinuation {
    pub source_fingerprint: String,
    pub anchor: LiveContactAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
}
impl Drop for LiveContactContinuation {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.search_query.zeroize();
    }
}
pub fn valid_contact_query(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.contains('\0')
}

/// A boundary in the local contact cache, not a server address-book cursor.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveContactAnchor {
    pub row_id: String,
    pub username: String,
}
impl LiveContactAnchor {
    pub fn key(&self) -> Option<i64> {
        let n = self.row_id.parse::<i64>().ok()?;
        (n > 0
            && n.to_string() == self.row_id
            && !self.username.is_empty()
            && self.username.len() <= 256
            && !self.username.chars().any(char::is_control))
        .then_some(n)
    }
}
impl Drop for LiveContactAnchor {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveContact {
    pub anchor: LiveContactAnchor,
    pub nickname: Option<String>,
    pub remark: Option<String>,
    pub alias: Option<String>,
    pub local_type: Option<i64>,
    pub delete_flag: Option<i64>,
    pub verify_flag: Option<i64>,
    /// Local cache flags; neither field asserts a current server relationship.
    #[serde(default)]
    pub contact_flag: Option<i64>,
    #[serde(default)]
    pub business_type: Option<i64>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}
impl LiveContact {
    pub fn matches_query(&self, query: &str) -> bool {
        let needle = query.to_lowercase();
        [
            Some(self.anchor.username.as_str()),
            self.nickname.as_deref(),
            self.remark.as_deref(),
            self.alias.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|value| value.to_lowercase().contains(&needle))
    }
    pub fn valid(&self) -> bool {
        self.anchor.key().is_some()
            && self
                .contact_flag
                .is_none_or(|n| (0..=9_007_199_254_740_991).contains(&n))
            && self
                .avatar_url
                .as_ref()
                .is_none_or(|s| contact_avatar_url(s).is_some())
            && self
                .description
                .as_ref()
                .is_none_or(|s| s.len() <= 4096 && !s.contains('\0'))
            && [&self.nickname, &self.remark, &self.alias].iter().all(|s| {
                s.as_ref()
                    .is_none_or(|s| s.len() <= 1024 && !s.contains('\0'))
            })
    }
}
impl Drop for LiveContact {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.nickname.zeroize();
        self.remark.zeroize();
        self.alias.zeroize();
        self.avatar_url.zeroize();
        self.description.zeroize();
    }
}

/// Contact metadata supplies the URL; public callers cannot choose an arbitrary host.
pub fn contact_avatar_url(value: &str) -> Option<String> {
    if value.len() > 4096
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
        || value.contains(['\\', '#'])
    {
        return None;
    }
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let host = host.to_ascii_lowercase();
    if !(host == "qlogo.cn" || host.ends_with(".qlogo.cn"))
        || !host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveContactWindow {
    pub schema_version: u32,
    pub rows: Vec<LiveContact>,
    pub has_more: bool,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl LiveContactWindow {
    pub fn valid(&self, after: Option<&LiveContactAnchor>) -> bool {
        if self.schema_version != 1
            || !self.read_only
            || !self.readonly_shm
            || self.rows.len() > LIVE_CONTACT_MAX_ROWS
            || (self.has_more && self.rows.len() != LIVE_CONTACT_MAX_ROWS)
            || after.is_some_and(|a| a.key().is_none())
        {
            return false;
        }
        let mut previous = after.and_then(LiveContactAnchor::key).unwrap_or(0);
        for row in &self.rows {
            if !row.valid() {
                return false;
            }
            let id = row.anchor.key().unwrap_or(0);
            if id <= previous {
                return false;
            }
            previous = id;
        }
        true
    }
}

pub const LIVE_SESSION_MAX_ROWS: usize = 200;
pub const LIVE_SESSION_MAX_BYTES: usize = 1024 * 1024;

/// Descending local session-cache order. This is not a server conversation cursor.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSessionAnchor {
    pub row_id: String,
    pub username: String,
    pub sort_timestamp: i64,
}
impl LiveSessionAnchor {
    pub fn key(&self) -> Option<(i64, i64)> {
        let n = self.row_id.parse::<i64>().ok()?;
        (n > 0
            && n.to_string() == self.row_id
            && self.sort_timestamp >= 0
            && !self.username.is_empty()
            && self.username.len() <= 256
            && !self.username.chars().any(char::is_control))
        .then_some((self.sort_timestamp, n))
    }
}
impl Drop for LiveSessionAnchor {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSessionContinuation {
    pub source_fingerprint: String,
    pub anchor: LiveSessionAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
}
impl Drop for LiveSessionContinuation {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.search_query.zeroize();
    }
}
pub fn valid_session_query(value: &str) -> bool {
    valid_contact_query(value)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSession {
    pub anchor: LiveSessionAnchor,
    pub summary: Option<String>,
    pub summary_status: String,
    pub last_timestamp: Option<i64>,
    pub unread_count: Option<i64>,
    pub is_hidden: Option<i64>,
}
impl LiveSession {
    pub fn matches_query(&self, query: &str) -> bool {
        let needle = query.to_lowercase();
        self.anchor.username.to_lowercase().contains(&needle)
            || self
                .summary
                .as_deref()
                .is_some_and(|s| s.to_lowercase().contains(&needle))
    }
    pub fn valid(&self) -> bool {
        self.anchor.key().is_some()
            && self.last_timestamp.is_none_or(|n| n >= 0)
            && self.unread_count.is_none_or(|n| n >= 0)
            && match (self.summary_status.as_str(), &self.summary) {
                ("text", Some(s)) => s.len() <= 4096 && !s.contains('\0'),
                ("missing" | "oversized" | "invalid", None) => true,
                _ => false,
            }
    }
}
impl Drop for LiveSession {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.summary.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSessionWindow {
    pub schema_version: u32,
    pub rows: Vec<LiveSession>,
    pub has_more: bool,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl LiveSessionWindow {
    pub fn valid(&self, after: Option<&LiveSessionAnchor>) -> bool {
        if self.schema_version != 1
            || !self.read_only
            || !self.readonly_shm
            || self.rows.len() > LIVE_SESSION_MAX_ROWS
            || (self.has_more && self.rows.len() != LIVE_SESSION_MAX_ROWS)
            || after.is_some_and(|a| a.key().is_none())
        {
            return false;
        }
        let mut previous = after.and_then(LiveSessionAnchor::key);
        for row in &self.rows {
            if !row.valid() {
                return false;
            }
            let key = row.anchor.key().expect("validated session");
            if previous.is_some_and(|p| key >= p) {
                return false;
            }
            previous = Some(key);
        }
        true
    }
}

/// A voice is selected from an authenticated conversation message window.
/// Media local IDs are not assumed to equal message-shard local IDs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveVoiceAnchor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub server_id: String,
    pub timestamp: i64,
}
impl LiveVoiceAnchor {
    pub fn conversation(&self) -> &str {
        self.username.as_deref().unwrap_or("filehelper")
    }
    pub fn server_key(&self) -> Option<i64> {
        let id = self.server_id.parse::<i64>().ok()?;
        (id > 0
            && id.to_string() == self.server_id
            && self.timestamp > 0
            && self.username.as_deref().is_none_or(valid_live_username))
        .then_some(id)
    }
}
impl Drop for LiveVoiceAnchor {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
        self.server_id.zeroize();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveVoicePayload {
    pub schema_version: u32,
    pub read_only: bool,
    pub silk_hex: Option<String>,
}
impl Drop for LiveVoicePayload {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.silk_hex.zeroize();
    }
}

/// A per-source boundary, validated again in the next short read transaction.
/// Public clients receive an authenticated token, never this raw worker input.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveAnchor {
    pub timestamp: i64,
    pub local_id: String,
    pub server_id: Option<String>,
}
impl LiveAnchor {
    pub fn key(&self) -> Option<(i64, i64)> {
        let id = self.local_id.parse::<i64>().ok()?;
        if self.timestamp < 0
            || id <= 0
            || id.to_string() != self.local_id
            || self
                .server_id
                .as_ref()
                .is_some_and(|s| s.parse::<i64>().ok().is_none_or(|n| n.to_string() != *s))
        {
            return None;
        }
        Some((self.timestamp, id))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveMessage {
    /// Exact recipients decoded from the optional source envelope, never display text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_user_list: Option<Vec<String>>,
    pub local_id: String,
    pub server_id: Option<String>,
    pub timestamp: i64,
    pub message_type: String,
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_username: Option<String>,
    pub content_status: String,
}
impl Drop for LiveMessage {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.local_id.zeroize();
        self.server_id.zeroize();
        self.text.zeroize();
        self.metadata.zeroize();
        self.sender_username.zeroize();
        self.at_user_list.zeroize();
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveWindow {
    pub schema_version: u32,
    pub rows: Vec<LiveMessage>,
    pub has_older: bool,
    pub read_only: bool,
    pub readonly_shm: bool,
    pub read_mode: String,
}

/// A message read target, never a send authorization or caller-provided SQL table.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveConversationSelection {
    pub username: String,
    pub before: Option<LiveAnchor>,
    /// Internal authenticated context read, including the existing boundary row.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_anchor: bool,
    /// Read the nearest 30 newer rows, returned in descending order.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub newer_context: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<LiveMessageQuery>,
}
impl LiveConversationSelection {
    pub fn valid(&self) -> bool {
        valid_live_username(&self.username)
            && self.before.as_ref().is_none_or(|a| a.key().is_some())
            && (!self.include_anchor || self.before.is_some())
            && (!self.newer_context
                || (!self.include_anchor
                    && (self.before.is_some()
                        || self
                            .query
                            .as_ref()
                            .is_some_and(|q| q.from_timestamp.is_some()))))
            && self.query.as_ref().is_none_or(LiveMessageQuery::valid)
    }
}

/// A source query, not a filter on the currently displayed page. Time bounds
/// use Unix seconds; the lower bound is inclusive and the upper is exclusive.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveMessageQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_timestamp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_timestamp: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub message_types: Vec<u32>,
    /// Direct appmsg subtypes; combined with all other filters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_types: Vec<u32>,
    /// Case-insensitive substring of decoded ordinary text (including Zstd).
    /// Media credentials and raw card XML are deliberately not searchable here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
}
impl LiveMessageQuery {
    pub fn valid(&self) -> bool {
        self.from_timestamp
            .is_none_or(|v| (0..253402300800).contains(&v))
            && self
                .until_timestamp
                .is_none_or(|v| (1..=253402300800).contains(&v))
            && !matches!((self.from_timestamp, self.until_timestamp), (Some(a), Some(b)) if a >= b)
            && self.app_types.len() <= 16
            && self
                .app_types
                .iter()
                .enumerate()
                .all(|(i, v)| *v > 0 && !self.app_types[..i].contains(v))
            && self.message_types.len() <= 16
            && self
                .message_types
                .iter()
                .enumerate()
                .all(|(i, v)| *v > 0 && !self.message_types[..i].contains(v))
            && self
                .text
                .as_ref()
                .is_none_or(|v| !v.trim().is_empty() && v.len() <= 256 && !v.contains('\0'))
            && self.server_id.as_ref().is_none_or(|v| {
                v.parse::<i64>()
                    .is_ok_and(|n| n != 0 && n.to_string() == *v)
            })
            && self
                .sender_username
                .as_ref()
                .is_none_or(|v| valid_live_username(v))
    }

    pub fn matches_content(&self, message_type: i64, text: Option<&str>) -> bool {
        self.matches_body(message_type, text, None)
    }
    pub fn matches_body(
        &self,
        message_type: i64,
        text: Option<&str>,
        sender: Option<&str>,
    ) -> bool {
        (self.message_types.is_empty() || self.message_types.contains(&(message_type as u32)))
            && (self.app_types.is_empty()
                || (message_type as u32 == 49
                    && text
                        .and_then(|body| query_body::app_type(body, sender))
                        .is_some_and(|kind| self.app_types.contains(&kind))))
            && self.text.as_ref().is_none_or(|needle| {
                message_type == 1
                    && text.is_some_and(|body| body.to_lowercase().contains(&needle.to_lowercase()))
            })
    }

    pub fn matches(&self, message: &LiveMessage) -> bool {
        self.from_timestamp.is_none_or(|v| message.timestamp >= v)
            && self.until_timestamp.is_none_or(|v| message.timestamp < v)
            && self
                .server_id
                .as_ref()
                .is_none_or(|id| message.server_id.as_ref() == Some(id))
            && self
                .sender_username
                .as_ref()
                .is_none_or(|sender| message.sender_username.as_ref() == Some(sender))
            && message.message_type.parse::<i64>().is_ok_and(|kind| {
                self.matches_body(
                    kind,
                    if kind == 1 {
                        message.text.as_deref()
                    } else {
                        message.metadata.as_deref()
                    },
                    message.sender_username.as_deref(),
                )
            })
    }
}
impl Drop for LiveMessageQuery {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.text.zeroize();
        self.server_id.zeroize();
        self.sender_username.zeroize();
    }
}

pub fn valid_live_username(username: &str) -> bool {
    !username.is_empty() && username.len() <= 256 && !username.chars().any(char::is_control)
}
impl Drop for LiveConversationSelection {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
    }
}
