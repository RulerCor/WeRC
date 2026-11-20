//! Bounded local group-cache observations, not server membership or send permission.
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;
pub const LIVE_GROUP_MAX_ROWS: usize = 200;
pub const LIVE_GROUP_MAX_BYTES: usize = 1024 * 1024;
fn valid_username(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
pub fn valid_group_username(s: &str) -> bool {
    valid_username(s) && s.len() > "@chatroom".len() && s.ends_with("@chatroom")
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LiveGroupAnchor {
    pub row_id: String,
    pub member_id: Option<String>,
    pub username: Option<String>,
}
impl LiveGroupAnchor {
    pub fn key(&self) -> Option<i64> {
        let id = self.row_id.parse::<i64>().ok()?;
        (id > 0
            && id.to_string() == self.row_id
            && self
                .member_id
                .as_ref()
                .is_none_or(|s| s.parse::<i64>().ok().is_some_and(|n| n.to_string() == *s))
            && self.username.as_deref().is_none_or(valid_username))
        .then_some(id)
    }
}
impl Drop for LiveGroupAnchor {
    fn drop(&mut self) {
        self.username.zeroize();
        self.member_id.zeroize();
        self.row_id.zeroize();
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveGroupSelection {
    pub username: String,
    pub after: Option<LiveGroupAnchor>,
}
impl LiveGroupSelection {
    pub fn valid(&self) -> bool {
        valid_group_username(&self.username)
            && self.after.as_ref().is_none_or(|a| a.key().is_some())
    }
}
impl Drop for LiveGroupSelection {
    fn drop(&mut self) {
        self.username.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveGroupContinuation {
    pub source_fingerprint: String,
    pub selection: LiveGroupSelection,
}
impl LiveGroupContinuation {
    pub fn valid(&self) -> bool {
        self.source_fingerprint.len() == 64
            && self
                .source_fingerprint
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            && self.selection.valid()
            && self.selection.after.is_some()
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveGroupMember {
    #[serde(default)]
    pub group_nickname: Option<String>,
    pub anchor: LiveGroupAnchor,
    pub nickname: Option<String>,
    pub remark: Option<String>,
}
impl LiveGroupMember {
    pub fn valid(&self) -> bool {
        self.anchor.key().is_some()
            && [&self.nickname, &self.remark, &self.group_nickname]
                .iter()
                .all(|s| {
                    s.as_ref()
                        .is_none_or(|s| s.len() <= 1024 && !s.contains('\0'))
                })
            && (self.anchor.username.is_some()
                || (self.nickname.is_none()
                    && self.remark.is_none()
                    && self.group_nickname.is_none()))
    }
}
impl Drop for LiveGroupMember {
    fn drop(&mut self) {
        self.nickname.zeroize();
        self.group_nickname.zeroize();
        self.remark.zeroize();
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveGroupPin {
    pub server_id: String,
}
impl LiveGroupPin {
    pub fn valid(&self) -> bool {
        self.server_id.parse::<u64>().ok().is_some_and(|n| n!=0 && n.to_string()==self.server_id)
    }
}
impl Drop for LiveGroupPin {fn drop(&mut self) {self.server_id.zeroize();}}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveGroupWindow {
    pub schema_version: u32,
    pub username: String,
    pub owner_username: Option<String>,
    #[serde(default)]
    pub cached_pins: Option<Vec<LiveGroupPin>>,
    pub rows: Vec<LiveGroupMember>,
    pub has_more: bool,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl LiveGroupWindow {
    pub fn valid(&self, selection: &LiveGroupSelection) -> bool {
        if !selection.valid()
            || self.schema_version != 1
            || self.username != selection.username
            || !self.read_only
            || !self.readonly_shm
            || self.rows.len() > LIVE_GROUP_MAX_ROWS
            || self.cached_pins.as_ref().is_some_and(|pins| {
                let mut ids=std::collections::BTreeSet::new();
                pins.len()>64 || pins.iter().any(|p| !p.valid() || !ids.insert(p.server_id.as_str()))
            })
            || (self.has_more && self.rows.len() != LIVE_GROUP_MAX_ROWS)
            || self
                .owner_username
                .as_deref()
                .is_some_and(|s| !valid_username(s))
        {
            return false;
        }
        let mut previous = selection
            .after
            .as_ref()
            .and_then(LiveGroupAnchor::key)
            .unwrap_or(0);
        for member in &self.rows {
            if !member.valid() {
                return false;
            }
            let id = member.anchor.key().unwrap_or(0);
            if id <= previous {
                return false;
            }
            previous = id;
        }
        true
    }
}
impl Drop for LiveGroupWindow {
    fn drop(&mut self) {
        self.username.zeroize();
        self.owner_username.zeroize();
    }
}
