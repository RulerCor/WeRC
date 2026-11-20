//! Bounded merge of independent short source transactions. Source IDs are never
//! collapsed into server-ID aliases; a merged window is not an atomic snapshot.
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use werc_domain::{AppError, AppResult, live::*};
use zeroize::Zeroizing;
pub(crate) const MAX_SOURCES: usize = 16;
fn invalid() -> AppError {
    AppError::new("LIVE_MULTI_INVALID", "跨分片窗口或游标无效")
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "跨分片窗口已过期或来源已变化")
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcedLiveMessage {
    /// Opaque per-view source token, independent of local/server message IDs.
    pub source: String,
    pub source_index: usize,
    pub message: LiveMessage,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceCursor {
    pub before: Option<LiveAnchor>,
    pub more: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MultiWindow {
    rows: Vec<SourcedLiveMessage>,
    next: Vec<SourceCursor>,
}
pub(crate) struct WindowMerger {
    rows: Vec<SourcedLiveMessage>,
    sources: Vec<(String, usize, SourceCursor, bool)>,
    limit: usize,
}
impl WindowMerger {
    pub fn new() -> Self {
        Self {
            rows: vec![],
            sources: vec![],
            limit: LIVE_WINDOW_MAX_ROWS,
        }
    }
    pub fn push(&mut self, tag: &str, window: LiveWindow, previous: SourceCursor) -> AppResult<()> {
        if self.sources.len() == MAX_SOURCES
            || tag.len() != 64
            || !tag.bytes().all(|b| b.is_ascii_hexdigit())
            || self.sources.iter().any(|s| s.0 == tag)
            || (window.has_older && window.rows.is_empty())
        {
            return Err(invalid());
        }
        drop(crate::live_view::encode_window(&window)?);
        if let Some(bound) = previous.before.as_ref() {
            let bound = bound.key().ok_or_else(invalid)?;
            if window
                .rows
                .iter()
                .any(|r| (r.timestamp, r.local_id.parse::<i64>().unwrap_or(i64::MAX)) >= bound)
            {
                return Err(invalid());
            }
        }
        // Every non-exhausted source must contribute at least the global result
        // budget. Reducing to the smallest such budget avoids hiding newer rows
        // of one shard behind older rows of a larger shard.
        if window.has_older {
            self.limit = self.limit.min(window.rows.len());
        }
        let index = self.sources.len();
        self.sources
            .push((tag.into(), window.rows.len(), previous, window.has_older));
        self.rows
            .extend(window.rows.into_iter().map(|message| SourcedLiveMessage {
                source: tag.into(),
                source_index: index + 1,
                message,
            }));
        self.rows.sort_by(|a, b| {
            (
                b.message.timestamp,
                &b.source,
                b.message.local_id.parse::<i64>().unwrap_or(0),
            )
                .cmp(&(
                    a.message.timestamp,
                    &a.source,
                    a.message.local_id.parse::<i64>().unwrap_or(0),
                ))
        });
        self.rows.truncate(self.limit);
        Ok(())
    }
    pub fn finish(self, binding: &str) -> AppResult<MultiReadView> {
        self.finish_with_context(binding, vec![])
    }
    pub fn finish_with_context(
        mut self,
        binding: &str,
        mut newer: Vec<SourcedLiveMessage>,
    ) -> AppResult<MultiReadView> {
        // Keep the closest 30 rows above the focus across ALL sources. Their
        // cursors must never replace the older per-source continuation anchors.
        newer.sort_by(|a, b| {
            (
                a.message.timestamp,
                &a.source,
                a.message.local_id.parse::<i64>().unwrap_or(0),
            )
                .cmp(&(
                    b.message.timestamp,
                    &b.source,
                    b.message.local_id.parse::<i64>().unwrap_or(0),
                ))
        });
        newer.truncate(30);
        self.rows.truncate(LIVE_WINDOW_MAX_ROWS - newer.len());
        if self.sources.is_empty() {
            return Err(invalid());
        }
        let next = self
            .sources
            .iter()
            .enumerate()
            .map(|(i, (_, count, previous, had_older))| {
                let included = self.rows.iter().filter(|r| r.source_index == i + 1).count();
                let before = self
                    .rows
                    .iter()
                    .rev()
                    .find(|r| r.source_index == i + 1)
                    .map(|r| LiveAnchor {
                        timestamp: r.message.timestamp,
                        local_id: r.message.local_id.clone(),
                        server_id: r.message.server_id.clone(),
                    })
                    .or_else(|| previous.before.clone());
                SourceCursor {
                    before,
                    more: *had_older || included < *count,
                }
            })
            .collect();
        newer.reverse();
        newer.append(&mut self.rows);
        MultiReadView::new(binding, MultiWindow { rows: newer, next })
    }
}
#[derive(Serialize)]
pub struct MultiLivePage {
    pub view_id: String,
    pub rows: Vec<SourcedLiveMessage>,
    pub source_count: usize,
    pub older: Option<String>,
    pub newer: Option<String>,
    pub continuation: Option<String>,
    pub expires_in_seconds: u64,
    pub mode: &'static str,
    pub source_snapshot_atomic: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MultiCursor {
    view: String,
    purpose: String,
    offset: usize,
}
pub(crate) struct MultiReadView {
    id: String,
    binding: String,
    created: Instant,
    key: Zeroizing<[u8; 32]>,
    sealed: Vec<u8>,
}
impl MultiReadView {
    pub(crate) fn matches_view(&self, id: &str) -> bool {
        self.id == id
    }
    pub(crate) fn check_view(&self, binding: &str, id: &str) -> AppResult<u64> {
        if self.id != id || self.binding != binding || self.expired() {
            return Err(stale());
        }
        Ok(120u64.saturating_sub(self.created.elapsed().as_secs()))
    }
    pub(crate) fn seal_group(
        &self,
        binding: &str,
        id: &str,
        value: &werc_domain::group::LiveGroupContinuation,
    ) -> AppResult<String> {
        self.check_view(binding, id)?;
        if !value.valid() {
            return Err(invalid());
        }
        Ok(hex::encode(self.seal("group-cursor", value)?))
    }
    pub(crate) fn open_group(
        &self,
        binding: &str,
        id: &str,
        cursor: &str,
    ) -> AppResult<werc_domain::group::LiveGroupContinuation> {
        self.check_view(binding, id)?;
        if !(56..=4096).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let value: werc_domain::group::LiveGroupContinuation =
            self.open(binding, "group-cursor", &bytes)?;
        if !value.valid() {
            return Err(stale());
        }
        Ok(value)
    }
    pub(crate) fn seal_contacts(
        &self,
        binding: &str,
        id: &str,
        value: &LiveContactContinuation,
    ) -> AppResult<String> {
        self.check_view(binding, id)?;
        validate_contact_cursor(value)?;
        Ok(hex::encode(self.seal("contacts-cursor", value)?))
    }
    pub(crate) fn open_contacts(
        &self,
        binding: &str,
        id: &str,
        cursor: &str,
    ) -> AppResult<LiveContactContinuation> {
        self.check_view(binding, id)?;
        if !(56..=8192).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let value = self.open(binding, "contacts-cursor", &bytes)?;
        validate_contact_cursor(&value)?;
        Ok(value)
    }
    pub(crate) fn seal_sessions(
        &self,
        binding: &str,
        id: &str,
        value: &LiveSessionContinuation,
    ) -> AppResult<String> {
        self.check_view(binding, id)?;
        validate_session_cursor(value)?;
        Ok(hex::encode(self.seal("sessions-cursor", value)?))
    }
    pub(crate) fn open_sessions(
        &self,
        binding: &str,
        id: &str,
        cursor: &str,
    ) -> AppResult<LiveSessionContinuation> {
        self.check_view(binding, id)?;
        if !(56..=8192).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let value = self.open(binding, "sessions-cursor", &bytes)?;
        validate_session_cursor(&value)?;
        Ok(value)
    }
    pub(crate) fn seal_search(
        &self,
        binding: &str,
        id: &str,
        value: &crate::live_search::SearchTicket,
    ) -> AppResult<String> {
        self.check_view(binding, id)?;
        Ok(hex::encode(self.seal("search-ticket", value)?))
    }
    pub(crate) fn open_search(
        &self,
        binding: &str,
        id: &str,
        cursor: &str,
    ) -> AppResult<crate::live_search::SearchTicket> {
        self.check_view(binding, id)?;
        if !(56..=8192).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let value = self.open(binding, "search-ticket", &bytes)?;
        Ok(value)
    }
    fn new(binding: &str, window: MultiWindow) -> AppResult<Self> {
        if binding.len() != 64 || !binding.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let mut key = Zeroizing::new([0; 32]);
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        let mut view = Self {
            id: uuid::Uuid::new_v4().to_string(),
            binding: binding.into(),
            created: Instant::now(),
            key,
            sealed: vec![],
        };
        view.sealed = view.seal("window", &window)?;
        Ok(view)
    }
    fn seal<T: Serialize>(&self, purpose: &str, value: &T) -> AppResult<Vec<u8>> {
        let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| invalid())?);
        if plain.len() > LIVE_WINDOW_MAX_BYTES {
            return Err(AppError::new(
                "LIVE_WINDOW_BYTES_LIMIT",
                "合并结果超过2MiB窗口上限",
            ));
        }
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = format!("WeRC/live-multi/v1/{}/{}/{purpose}", self.id, self.binding);
        let encrypted = Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| invalid())?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        Ok([nonce.to_vec(), encrypted].concat())
    }
    fn open<T: serde::de::DeserializeOwned>(
        &self,
        binding: &str,
        purpose: &str,
        bytes: &[u8],
    ) -> AppResult<T> {
        if self.expired() || self.binding != binding {
            return Err(stale());
        }
        if !(28..=LIVE_WINDOW_MAX_BYTES + 28).contains(&bytes.len()) {
            return Err(invalid());
        }
        let aad = format!("WeRC/live-multi/v1/{}/{}/{purpose}", self.id, self.binding);
        let plain = Zeroizing::new(
            Aes256Gcm::new_from_slice(self.key.as_ref())
                .map_err(|_| invalid())?
                .decrypt(
                    Nonce::from_slice(&bytes[..12]),
                    Payload {
                        msg: &bytes[12..],
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| invalid())?,
        );
        serde_json::from_slice(&plain).map_err(|_| invalid())
    }
    fn token(&self, purpose: &str, offset: usize) -> AppResult<String> {
        Ok(hex::encode(self.seal(
            "cursor",
            &MultiCursor {
                view: self.id.clone(),
                purpose: purpose.into(),
                offset,
            },
        )?))
    }
    fn cursor(&self, binding: &str, text: &str, purpose: &str) -> AppResult<usize> {
        if !(56..=2048).contains(&text.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(text).map_err(|_| stale())?;
        let cursor: MultiCursor = self.open(binding, "cursor", &bytes)?;
        if cursor.view != self.id || cursor.purpose != purpose {
            return Err(stale());
        }
        Ok(cursor.offset)
    }
    pub fn expired(&self) -> bool {
        self.created.elapsed() >= Duration::from_secs(120)
    }
    pub(crate) fn recovery_window(
        &self,
        binding: &str,
        id: &str,
    ) -> AppResult<(Vec<SourcedLiveMessage>, Vec<bool>)> {
        if self.id != id {
            return Err(stale());
        }
        let window: MultiWindow = self.open(binding, "window", &self.sealed)?;
        Ok((window.rows, window.next.iter().map(|s| s.more).collect()))
    }
    pub fn page(
        &self,
        binding: &str,
        cursor: Option<(&str, crate::live_view::LiveDirection)>,
        limit: usize,
    ) -> AppResult<MultiLivePage> {
        if !(1..=100).contains(&limit) {
            return Err(AppError::new("INVALID_ARGUMENT", "每页需要1至100条"));
        }
        let window: MultiWindow = self.open(binding, "window", &self.sealed)?;
        let offset = if let Some((text, direction)) = cursor {
            self.cursor(
                binding,
                text,
                match direction {
                    crate::live_view::LiveDirection::Older => "older",
                    crate::live_view::LiveDirection::Newer => "newer",
                },
            )?
        } else {
            0
        };
        if offset > window.rows.len() || (offset == window.rows.len() && offset > 0) {
            return Err(stale());
        }
        let end = (offset + limit).min(window.rows.len());
        Ok(MultiLivePage {
            view_id: self.id.clone(),
            source_count: window.next.len(),
            rows: window.rows[offset..end].to_vec(),
            older: if end < window.rows.len() {
                Some(self.token("older", end)?)
            } else {
                None
            },
            newer: if offset > 0 {
                Some(self.token("newer", offset.saturating_sub(limit))?)
            } else {
                None
            },
            continuation: if end == window.rows.len() && window.next.iter().any(|s| s.more) {
                Some(self.token("sources", end)?)
            } else {
                None
            },
            expires_in_seconds: 120u64.saturating_sub(self.created.elapsed().as_secs()),
            mode: "live_multi_source_window",
            source_snapshot_atomic: false,
        })
    }
    pub fn continuation(&self, binding: &str, cursor: &str) -> AppResult<Vec<SourceCursor>> {
        let end = self.cursor(binding, cursor, "sources")?;
        let window: MultiWindow = self.open(binding, "window", &self.sealed)?;
        if end != window.rows.len() || !window.next.iter().any(|s| s.more) {
            return Err(stale());
        }
        Ok(window.next)
    }
}

fn validate_contact_cursor(value: &LiveContactContinuation) -> AppResult<()> {
    if value.anchor.key().is_none()
        || value
            .search_query
            .as_deref()
            .is_some_and(|q| !werc_domain::live::valid_contact_query(q))
        || value.source_fingerprint.len() != 64
        || !value
            .source_fingerprint
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_session_cursor(value: &LiveSessionContinuation) -> AppResult<()> {
    if value.anchor.key().is_none()
        || value
            .search_query
            .as_deref()
            .is_some_and(|q| !werc_domain::live::valid_session_query(q))
        || value.source_fingerprint.len() != 64
        || !value
            .source_fingerprint
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window(base: i64, count: usize, older: bool) -> LiveWindow {
        LiveWindow {
            schema_version: 1,
            rows: (0..count)
                .map(|n| LiveMessage {
                    at_user_list: None,
                    local_id: (count - n).to_string(),
                    server_id: Some("7".into()),
                    timestamp: base - n as i64,
                    message_type: "1".into(),
                    text: Some("owned".into()),
                    content_status: "text".into(),
                    metadata: None,
                    sender_username: None,
                })
                .collect(),
            has_older: older,
            read_only: true,
            readonly_shm: true,
            read_mode: "bounded_transaction_window".into(),
        }
    }
    fn start() -> SourceCursor {
        SourceCursor {
            before: None,
            more: true,
        }
    }
    #[test]
    fn skewed_shards_choose_global_newest_and_keep_independent_continuation() {
        let mut merge = WindowMerger::new();
        let context = "a".repeat(64);
        merge
            .push(&"1".repeat(64), window(1000, 120, true), start())
            .unwrap();
        merge
            .push(&"2".repeat(64), window(500, 200, true), start())
            .unwrap();
        let view = merge.finish(&context).unwrap();
        let first = view.page(&context, None, 60).unwrap();
        assert!(first.rows.iter().all(|r| r.source_index == 1));
        let second = view
            .page(
                &context,
                Some((
                    first.older.as_deref().unwrap(),
                    crate::live_view::LiveDirection::Older,
                )),
                60,
            )
            .unwrap();
        let anchors = view
            .continuation(&context, second.continuation.as_deref().unwrap())
            .unwrap();
        assert_eq!(anchors[0].before.as_ref().unwrap().local_id, "1");
        assert!(anchors[1].before.is_none());
        assert!(anchors.iter().all(|s| s.more));
        assert!(
            view.continuation(&context, first.older.as_deref().unwrap())
                .is_err()
        );
        assert!(view.page(&"b".repeat(64), None, 60).is_err());
    }
    #[test]
    fn sender_mapping_stays_with_source_even_when_message_ids_overlap() {
        let mut merge = WindowMerger::new();
        for (source, sender) in [("1", "owned-sender-one"), ("2", "owned-sender-two")] {
            let mut data = window(1000, 2, false);
            data.rows[0].sender_username = Some(sender.into());
            merge.push(&source.repeat(64), data, start()).unwrap();
        }
        let context = "a".repeat(64);
        let view = merge.finish(&context).unwrap();
        let page = view.page(&context, None, 60).unwrap();
        for row in &page.rows {
            assert_eq!(
                row.message.sender_username.as_deref(),
                if row.message.local_id == "2" {
                    Some(if row.source_index == 1 {
                        "owned-sender-one"
                    } else {
                        "owned-sender-two"
                    })
                } else {
                    None
                }
            );
        }
        assert!(view.page(&"b".repeat(64), None, 60).is_err());
    }

    #[test]
    fn duplicate_local_and_server_ids_across_sources_remain_distinct_and_sorted() {
        let mut merge = WindowMerger::new();
        let context = "a".repeat(64);
        merge
            .push(&"1".repeat(64), window(1000, 4, false), start())
            .unwrap();
        merge
            .push(&"2".repeat(64), window(1000, 4, false), start())
            .unwrap();
        let mut view = merge.finish(&context).unwrap();
        let page = view.page(&context, None, 60).unwrap();
        assert_eq!(page.rows.len(), 8);
        assert_eq!(page.rows[0].source_index, 2);
        assert_eq!(page.rows[1].source_index, 1);
        assert!(page.continuation.is_none());
        assert!(!page.source_snapshot_atomic);
        view.created -= Duration::from_secs(121);
        assert!(view.page(&context, None, 60).is_err());
    }
    #[test]
    fn merge_rejects_source_reuse_invalid_order_and_backwards_boundary_violation() {
        let mut merge = WindowMerger::new();
        let tag = "1".repeat(64);
        merge.push(&tag, window(100, 3, false), start()).unwrap();
        assert!(merge.push(&tag, window(100, 3, false), start()).is_err());
        let mut bad = window(100, 3, false);
        bad.rows.reverse();
        assert!(merge.push(&"2".repeat(64), bad, start()).is_err());
        assert!(
            merge
                .push(
                    &"2".repeat(64),
                    window(100, 3, false),
                    SourceCursor {
                        before: Some(LiveAnchor {
                            timestamp: 99,
                            local_id: "9".into(),
                            server_id: None
                        }),
                        more: true
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn group_cursor_binds_identity_purpose_and_expiration() {
        use werc_domain::group::*;
        let binding = "a".repeat(64);
        let mut merge = WindowMerger::new();
        merge
            .push(&"1".repeat(64), window(1000, 120, true), start())
            .unwrap();
        let mut view = merge.finish(&binding).unwrap();
        let value = LiveGroupContinuation {
            source_fingerprint: "c".repeat(64),
            selection: LiveGroupSelection {
                username: "owned@chatroom".into(),
                after: Some(LiveGroupAnchor {
                    row_id: "9007199254740993".into(),
                    member_id: Some("4".into()),
                    username: Some("owned-member".into()),
                }),
            },
        };
        let token = view.seal_group(&binding, &view.id, &value).unwrap();
        assert!(!String::from_utf8_lossy(&hex::decode(&token).unwrap()).contains("owned"));
        assert_eq!(
            view.open_group(&binding, &view.id, &token)
                .unwrap()
                .selection
                .username,
            "owned@chatroom"
        );
        assert!(view.open_group(&binding, "wrong", &token).is_err());
        assert!(view.open_group(&"b".repeat(64), &view.id, &token).is_err());
        assert!(view.open_sessions(&binding, &view.id, &token).is_err());
        let message = view.page(&binding, None, 60).unwrap().older.unwrap();
        assert!(view.open_group(&binding, &view.id, &message).is_err());
        let mut bad = token.clone().into_bytes();
        bad[20] = if bad[20] == b'a' { b'b' } else { b'a' };
        assert!(
            view.open_group(&binding, &view.id, std::str::from_utf8(&bad).unwrap())
                .is_err()
        );
        view.created = Instant::now() - Duration::from_secs(121);
        assert!(view.open_group(&binding, &view.id, &token).is_err());
        assert!(view.seal_group(&binding, &view.id, &value).is_err());
    }
}
