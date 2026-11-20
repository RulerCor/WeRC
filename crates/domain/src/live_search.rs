//! Bounded, source-local cross-conversation search. Cursors are sealed by the application.
use super::*;

pub const LIVE_SEARCH_MAX_ROWS: usize = 60;
pub const LIVE_SEARCH_MAX_CONVERSATIONS: usize = 16;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSearchPosition {
    pub username: String,
    pub before: Option<LiveAnchor>,
}
impl LiveSearchPosition {
    pub fn valid(&self) -> bool {
        valid_live_username(&self.username)
            && self.before.as_ref().is_none_or(|a| a.key().is_some())
    }
}
impl Drop for LiveSearchPosition {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSearchSelection {
    pub query: LiveMessageQuery,
    pub after: Option<LiveSearchPosition>,
}
impl LiveSearchSelection {
    pub fn valid(&self) -> bool {
        self.query.valid() && self.after.as_ref().is_none_or(LiveSearchPosition::valid)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSearchHit {
    pub username: String,
    pub message: LiveMessage,
}
impl Drop for LiveSearchHit {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.username.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSearchWindow {
    pub rows: Vec<LiveSearchHit>,
    pub next: Option<LiveSearchPosition>,
    pub scanned_conversations: usize,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl LiveSearchWindow {
    pub fn valid(&self, selection: &LiveSearchSelection) -> bool {
        if !selection.valid()
            || !self.read_only
            || !self.readonly_shm
            || self.rows.len() > LIVE_SEARCH_MAX_ROWS
            || self.scanned_conversations > LIVE_SEARCH_MAX_CONVERSATIONS
            || self.next.as_ref().is_some_and(|p| !p.valid())
        {
            return false;
        }
        let mut previous: Option<(&str, (i64, i64))> = None;
        for hit in &self.rows {
            let row = &hit.message;
            let Some(key) = (LiveAnchor {
                timestamp: row.timestamp,
                local_id: row.local_id.clone(),
                server_id: row.server_id.clone(),
            })
            .key() else {
                return false;
            };
            if !valid_live_username(&hit.username)
                || !selection.query.matches(row)
                || row.message_type.parse::<i64>().is_err()
                || row.at_user_list.as_ref().is_some_and(|users| {
                    users.len() > 256 || users.iter().any(|s| !valid_live_username(s))
                })
                || row
                    .text
                    .as_ref()
                    .is_some_and(|s| s.len() > 16384 || s.contains('\0'))
                || row
                    .metadata
                    .as_ref()
                    .is_some_and(|s| s.len() > 16384 || s.contains('\0'))
                || row
                    .sender_username
                    .as_deref()
                    .is_some_and(|s| !valid_live_username(s))
                || previous.is_some_and(|(name, prior)| {
                    hit.username.as_str() < name || (hit.username == name && key >= prior)
                })
                || selection.after.as_ref().is_some_and(|p| {
                    hit.username < p.username
                        || (hit.username == p.username
                            && p.before
                                .as_ref()
                                .is_none_or(|a| a.key().is_none_or(|k| key >= k)))
                })
            {
                return false;
            }
            previous = Some((&hit.username, key));
        }
        if let Some(next) = &self.next {
            if selection.after.as_ref().is_some_and(|p| {
                next.username < p.username
                    || (next.username == p.username
                        && p.before.as_ref().is_none_or(|a| {
                            next.before.as_ref().is_some_and(|b| b.key() >= a.key())
                        }))
            }) {
                return false;
            }
            if let Some(last) = self.rows.last() {
                if next.username < last.username {
                    return false;
                }
                if let Some(anchor) = &next.before {
                    if next.username != last.username
                        || anchor.local_id != last.message.local_id
                        || anchor.timestamp != last.message.timestamp
                        || anchor.server_id != last.message.server_id
                    {
                        return false;
                    }
                }
            } else if next.before.is_some() {
                return false;
            }
        }
        true
    }
}
