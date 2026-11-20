//! Local WeChat Favorites. Wire rows are internal; only display projections reach clients.
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;
#[path = "favorites/content.rs"]
mod content;
pub use content::*;
pub const MAX_ROWS: usize = 30;
pub const MAX_XML: usize = 262144;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FavoriteAnchor {
    pub updated: i64,
    pub local_id: String,
}
impl FavoriteAnchor {
    pub fn key(&self) -> Option<(i64, i64)> {
        let id = self.local_id.parse::<i64>().ok()?;
        (id > 0 && id.to_string() == self.local_id && (0..253402300800).contains(&self.updated))
            .then_some((self.updated, id))
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FavoriteQuery {
    pub kind: Option<u32>,
    #[serde(default)]
    pub text: String,
    pub tag: Option<String>,
    pub from: Option<i64>,
    pub until: Option<i64>,
}
impl FavoriteQuery {
    pub fn valid(&self) -> bool {
        self.kind.is_none_or(|v| v > 0)
            && self.text.len() <= 512
            && !self.text.chars().any(char::is_control)
            && self
                .tag
                .as_ref()
                .is_none_or(|s| s.len() <= 256 && !s.chars().any(char::is_control))
            && self.from.is_none_or(|n| (0..253402300800).contains(&n))
            && self.until.is_none_or(|n| (1..=253402300800).contains(&n))
            && !matches!((self.from,self.until),(Some(a),Some(b)) if a>=b)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FavoriteSelection {
    pub query: FavoriteQuery,
    pub after: Option<FavoriteAnchor>,
    #[serde(default)]
    pub exact: bool,
}
impl FavoriteSelection {
    pub fn valid(&self) -> bool {
        self.query.valid()
            && self.after.as_ref().is_none_or(|a| a.key().is_some())
            && (!self.exact || self.after.is_some())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FavoriteRaw {
    pub anchor: FavoriteAnchor,
    pub server_id: String,
    pub kind: u32,
    pub source_user: Option<String>,
    pub conversation: Option<String>,
    pub xml: Option<String>,
    pub content_status: String,
    /// Present when the local tag relationship tables were read. Even an empty
    /// list overrides XML, which can retain tags removed after synchronization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}
impl FavoriteRaw {
    pub fn display_content(&self) -> Option<FavoriteContent> {
        let mut content = parse_content(self.xml.as_deref()?, self.kind).ok()?;
        if let Some(tags) = &self.tags {
            content.tags = tags.clone();
        }
        Some(content)
    }
}
impl Drop for FavoriteRaw {
    fn drop(&mut self) {
        self.source_user.zeroize();
        self.conversation.zeroize();
        self.xml.zeroize();
        self.tags.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FavoriteWindow {
    pub rows: Vec<FavoriteRaw>,
    pub next: Option<FavoriteAnchor>,
    pub counts: std::collections::BTreeMap<u32, u64>,
    pub scanned: usize,
    pub read_only: bool,
    pub readonly_shm: bool,
}
impl FavoriteWindow {
    pub fn valid(&self, s: &FavoriteSelection) -> bool {
        if !s.valid()
            || !self.read_only
            || !self.readonly_shm
            || self.rows.len() > MAX_ROWS
            || self.scanned > 200
            || self.scanned < self.rows.len()
            || self.counts.len() > 128
        {
            return false;
        }
        let mut prev = if s.exact {
            None
        } else {
            s.after.as_ref().and_then(FavoriteAnchor::key)
        };
        for r in &self.rows {
            let Some(k) = r.anchor.key() else {
                return false;
            };
            if prev.is_some_and(|v| k >= v)
                || r.server_id.parse::<u64>().is_err()
                || r.kind == 0
                || r.xml.as_ref().is_some_and(|x| x.len() > MAX_XML)
                || r.source_user.as_ref().is_some_and(|v| v.len() > 256)
                || r.conversation.as_ref().is_some_and(|v| v.len() > 256)
                || r.tags.as_ref().is_some_and(|tags| tags.len() > 128 || tags.iter().any(|tag|
                    tag.is_empty() || tag.len() > 256 || tag.chars().any(char::is_control)))
                || !matches!(
                    r.content_status.as_str(),
                    "parsed" | "invalid" | "missing" | "oversized" | "unsupported"
                )
            {
                return false;
            }
            if s.exact && s.after.as_ref().and_then(FavoriteAnchor::key) != Some(k) {
                return false;
            }
            prev = Some(k);
        }
        if s.exact && (self.rows.len() > 1 || self.next.is_some()) {
            return false;
        }
        self.next.as_ref().is_none_or(|a| {
            a.key().is_some_and(|k| {
                s.after
                    .as_ref()
                    .and_then(FavoriteAnchor::key)
                    .is_none_or(|v| k < v)
                    && prev.is_none_or(|v| k <= v)
            })
        })
    }
}
