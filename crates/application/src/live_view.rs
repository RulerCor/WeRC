//! Expiring encrypted result views. They hold no SQLite connection or source lock.
//! The caller must revalidate the source/login/connection binding before each page.
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use werc_domain::{AppError, AppResult, live::*};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveDirection {
    Older,
    Newer,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    view: String,
    purpose: String,
    timestamp: i64,
    local_id: String,
}

#[derive(Serialize)]
pub struct LivePage {
    pub view_id: String,
    pub rows: Vec<LiveMessage>,
    pub older: Option<String>,
    pub newer: Option<String>,
    pub continuation: Option<String>,
    pub older_source_rows_outside_window: bool,
    pub expires_in_seconds: u64,
    pub mode: &'static str,
}

pub struct LiveReadView {
    id: String,
    binding: String,
    created: Instant,
    key: Zeroizing<[u8; 32]>,
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

fn stale() -> AppError {
    AppError::new(
        "CURSOR_STALE",
        "在线读取视图已过期、身份已变化或游标无效，请重新读取",
    )
}
fn invalid() -> AppError {
    AppError::new("LIVE_VIEW_INVALID", "在线读取视图认证或格式无效")
}

pub(crate) fn encode_window(window: &LiveWindow) -> AppResult<Zeroizing<Vec<u8>>> {
    if window.rows.len() > LIVE_WINDOW_MAX_ROWS
        || window.schema_version != 1
        || !window.read_only
        || !window.readonly_shm
        || window.read_mode != "bounded_transaction_window"
    {
        return Err(invalid());
    }
    let mut previous = None;
    for row in &window.rows {
        let id = row
            .local_id
            .parse::<i64>()
            .ok()
            .filter(|v| *v > 0 && v.to_string() == row.local_id)
            .ok_or_else(invalid)?;
        if row.timestamp < 0
            || previous.is_some_and(|p| (row.timestamp, id) >= p)
            || row.server_id.as_ref().is_some_and(|s| s.len() > 20)
            || row.message_type.len() > 20
            || row.at_user_list.as_ref().is_some_and(|users| users.len()>256 || users.iter().any(|s| !werc_domain::live::valid_live_username(s)))
            || row.content_status.len() > 32
            || row
                .sender_username
                .as_ref()
                .is_some_and(|s| !valid_live_username(s))
            || row.text.as_ref().is_some_and(|t| t.len() > 16384)
            || row
                .metadata
                .as_ref()
                .is_some_and(|t| t.len() > 16384 || t.contains('\0'))
        {
            return Err(invalid());
        }
        previous = Some((row.timestamp, id));
    }
    let plain = Zeroizing::new(serde_json::to_vec(window).map_err(|_| invalid())?);
    if plain.len() > LIVE_WINDOW_MAX_BYTES {
        return Err(invalid());
    }
    Ok(plain)
}

impl LiveReadView {
    pub(crate) fn matches_view(&self, id: &str) -> bool {
        self.id == id
    }
    pub fn new(binding: &str, window: LiveWindow) -> AppResult<Self> {
        if binding.len() != 64 || !binding.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let plain = encode_window(&window)?;
        let mut key = Zeroizing::new([0; 32]);
        let mut nonce = [0; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let id = uuid::Uuid::new_v4().to_string();
        let aad = format!("WeRC/live-view/v1/{id}/{binding}");
        let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|_| invalid())?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        Ok(Self {
            id,
            binding: binding.into(),
            created: Instant::now(),
            key,
            nonce,
            ciphertext,
        })
    }
    pub fn expired(&self) -> bool {
        self.created.elapsed() >= Duration::from_secs(120)
    }
    pub(crate) fn check_view(&self, binding: &str, view_id: &str) -> AppResult<u64> {
        if self.expired() || binding != self.binding || view_id != self.id {
            return Err(stale());
        }
        Ok(120u64.saturating_sub(self.created.elapsed().as_secs()))
    }
    pub(crate) fn seal_contacts(
        &self,
        binding: &str,
        view_id: &str,
        value: &LiveContactContinuation,
    ) -> AppResult<String> {
        self.check_view(binding, view_id)?;
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
        let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| invalid())?);
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = format!("WeRC/live-contacts-cursor/v1/{}/{}", self.id, self.binding);
        let encrypted = self
            .cipher()?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        Ok(format!("{}{}", hex::encode(nonce), hex::encode(encrypted)))
    }
    pub(crate) fn open_contacts(
        &self,
        binding: &str,
        view_id: &str,
        cursor: &str,
    ) -> AppResult<LiveContactContinuation> {
        self.check_view(binding, view_id)?;
        if !(56..=8192).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let aad = format!("WeRC/live-contacts-cursor/v1/{}/{}", self.id, self.binding);
        let plain = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    Nonce::from_slice(&bytes[..12]),
                    Payload {
                        msg: &bytes[12..],
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| stale())?,
        );
        let value: LiveContactContinuation = serde_json::from_slice(&plain).map_err(|_| stale())?;
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
            return Err(stale());
        }
        Ok(value)
    }
    fn cipher(&self) -> AppResult<Aes256Gcm> {
        Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| invalid())
    }
    fn seal(&self, purpose: &str, row: &LiveMessage) -> AppResult<String> {
        let plain = Zeroizing::new(
            serde_json::to_vec(&Cursor {
                view: self.id.clone(),
                purpose: purpose.into(),
                timestamp: row.timestamp,
                local_id: row.local_id.clone(),
            })
            .map_err(|_| invalid())?,
        );
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = format!("WeRC/live-cursor/v1/{}/{}", self.id, self.binding);
        let encrypted = self
            .cipher()?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        Ok(format!("{}{}", hex::encode(nonce), hex::encode(encrypted)))
    }
    pub(crate) fn seal_group(
        &self,
        binding: &str,
        view_id: &str,
        value: &werc_domain::group::LiveGroupContinuation,
    ) -> AppResult<String> {
        self.check_view(binding, view_id)?;
        if !value.valid() {
            return Err(invalid());
        }
        let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| invalid())?);
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = format!("WeRC/live-group-cursor/v1/{}/{}", self.id, self.binding);
        let encrypted = self
            .cipher()?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        Ok(format!("{}{}", hex::encode(nonce), hex::encode(encrypted)))
    }
    pub(crate) fn open_group(
        &self,
        binding: &str,
        view_id: &str,
        cursor: &str,
    ) -> AppResult<werc_domain::group::LiveGroupContinuation> {
        self.check_view(binding, view_id)?;
        if !(56..=4096).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let aad = format!("WeRC/live-group-cursor/v1/{}/{}", self.id, self.binding);
        let plain = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    Nonce::from_slice(&bytes[..12]),
                    Payload {
                        msg: &bytes[12..],
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| stale())?,
        );
        let value: werc_domain::group::LiveGroupContinuation =
            serde_json::from_slice(&plain).map_err(|_| stale())?;
        if !value.valid() {
            return Err(stale());
        }
        Ok(value)
    }
    pub(crate) fn seal_sessions(
        &self,
        binding: &str,
        view_id: &str,
        value: &LiveSessionContinuation,
    ) -> AppResult<String> {
        self.check_view(binding, view_id)?;
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
        let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| invalid())?);
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = format!("WeRC/live-sessions-cursor/v1/{}/{}", self.id, self.binding);
        let encrypted = self
            .cipher()?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        Ok(format!("{}{}", hex::encode(nonce), hex::encode(encrypted)))
    }
    pub(crate) fn open_sessions(
        &self,
        binding: &str,
        view_id: &str,
        cursor: &str,
    ) -> AppResult<LiveSessionContinuation> {
        self.check_view(binding, view_id)?;
        if !(56..=8192).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let aad = format!("WeRC/live-sessions-cursor/v1/{}/{}", self.id, self.binding);
        let plain = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    Nonce::from_slice(&bytes[..12]),
                    Payload {
                        msg: &bytes[12..],
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| stale())?,
        );
        let value: LiveSessionContinuation = serde_json::from_slice(&plain).map_err(|_| stale())?;
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
            return Err(stale());
        }
        Ok(value)
    }
    pub(crate) fn seal_search(
        &self,
        binding: &str,
        view_id: &str,
        value: &crate::live_search::SearchTicket,
    ) -> AppResult<String> {
        self.check_view(binding, view_id)?;
        let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| invalid())?);
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let aad = format!("WeRC/live-search-ticket/v1/{}/{}", self.id, self.binding);
        let encrypted = self
            .cipher()?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| invalid())?;
        Ok(format!("{}{}", hex::encode(nonce), hex::encode(encrypted)))
    }
    pub(crate) fn open_search(
        &self,
        binding: &str,
        view_id: &str,
        cursor: &str,
    ) -> AppResult<crate::live_search::SearchTicket> {
        self.check_view(binding, view_id)?;
        if !(56..=8192).contains(&cursor.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(cursor).map_err(|_| stale())?;
        let aad = format!("WeRC/live-search-ticket/v1/{}/{}", self.id, self.binding);
        let plain = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    Nonce::from_slice(&bytes[..12]),
                    Payload {
                        msg: &bytes[12..],
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| stale())?,
        );
        let value: crate::live_search::SearchTicket =
            serde_json::from_slice(&plain).map_err(|_| stale())?;
        Ok(value)
    }
    pub fn page(
        &self,
        binding: &str,
        cursor: Option<(&str, LiveDirection)>,
        limit: usize,
    ) -> AppResult<LivePage> {
        if self.expired() || binding != self.binding {
            return Err(stale());
        }
        if !(1..=100).contains(&limit) {
            return Err(AppError::new("INVALID_ARGUMENT", "每页需要1至100条"));
        }
        let window = self.window(binding)?;
        let (start, end) = if let Some((text, direction)) = cursor {
            let purpose = match direction {
                LiveDirection::Older => "page_older",
                LiveDirection::Newer => "page_newer",
            };
            let cursor = self.cursor(binding, text, purpose)?;
            let anchor = window
                .rows
                .iter()
                .position(|r| r.timestamp == cursor.timestamp && r.local_id == cursor.local_id)
                .ok_or_else(stale)?;
            match direction {
                LiveDirection::Older => {
                    let start = anchor + 1;
                    (start, (start + limit).min(window.rows.len()))
                }
                LiveDirection::Newer => (anchor.saturating_sub(limit), anchor),
            }
        } else {
            (0, limit.min(window.rows.len()))
        };
        let rows = window.rows[start..end].to_vec();
        let older = if end < window.rows.len() {
            rows.last()
                .map(|r| self.seal("page_older", r))
                .transpose()?
        } else {
            None
        };
        let newer = if start > 0 {
            rows.first()
                .map(|r| self.seal("page_newer", r))
                .transpose()?
        } else {
            None
        };
        let continuation = if end == window.rows.len() && window.has_older {
            rows.last()
                .map(|r| self.seal("source_older", r))
                .transpose()?
        } else {
            None
        };
        Ok(LivePage {
            view_id: self.id.clone(),
            rows,
            older,
            newer,
            continuation,
            older_source_rows_outside_window: window.has_older,
            expires_in_seconds: 120_u64.saturating_sub(self.created.elapsed().as_secs()),
            mode: "live_bounded_result_view",
        })
    }
    pub(crate) fn recovery_window(&self, binding: &str, view_id: &str) -> AppResult<LiveWindow> {
        if self.id != view_id {
            return Err(stale());
        }
        self.window(binding)
    }
    fn window(&self, binding: &str) -> AppResult<LiveWindow> {
        if self.expired() || binding != self.binding {
            return Err(stale());
        }
        let aad = format!("WeRC/live-view/v1/{}/{}", self.id, self.binding);
        let plain = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    Nonce::from_slice(&self.nonce),
                    Payload {
                        msg: &self.ciphertext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| invalid())?,
        );
        serde_json::from_slice(&plain).map_err(|_| invalid())
    }
    fn cursor(&self, binding: &str, text: &str, purpose: &str) -> AppResult<Cursor> {
        if self.expired() || binding != self.binding || !(56..=2048).contains(&text.len()) {
            return Err(stale());
        }
        let bytes = hex::decode(text).map_err(|_| stale())?;
        let aad = format!("WeRC/live-cursor/v1/{}/{}", self.id, self.binding);
        let decoded = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    Nonce::from_slice(&bytes[..12]),
                    Payload {
                        msg: &bytes[12..],
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| stale())?,
        );
        let cursor: Cursor = serde_json::from_slice(&decoded).map_err(|_| stale())?;
        if cursor.view != self.id || cursor.purpose != purpose {
            return Err(stale());
        }
        Ok(cursor)
    }
    pub fn continuation_anchor(&self, binding: &str, text: &str) -> AppResult<LiveAnchor> {
        let cursor = self.cursor(binding, text, "source_older")?;
        let window = self.window(binding)?;
        let row = window
            .rows
            .last()
            .filter(|r| {
                window.has_older && r.timestamp == cursor.timestamp && r.local_id == cursor.local_id
            })
            .ok_or_else(stale)?;
        Ok(LiveAnchor {
            timestamp: row.timestamp,
            local_id: row.local_id.clone(),
            server_id: row.server_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window(count: usize) -> LiveWindow {
        LiveWindow {
            schema_version: 1,
            rows: (1..=count)
                .rev()
                .map(|id| LiveMessage {
                    at_user_list: None,
                    local_id: id.to_string(),
                    server_id: Some((9007199254740992_u64 + id as u64).to_string()),
                    timestamp: 1700000000,
                    message_type: "1".into(),
                    text: Some("owned-view-message".into()),
                    content_status: "text".into(),
                    metadata: None,
                    sender_username: None,
                })
                .collect(),
            has_older: true,
            read_only: true,
            readonly_shm: true,
            read_mode: "bounded_transaction_window".into(),
        }
    }
    #[test]
    fn sender_survives_encrypted_view_and_legacy_json_remains_unchanged() {
        let binding = "a".repeat(64);
        let old = br#"{"local_id":"1","server_id":null,"timestamp":1,"message_type":"1","text":"owned","content_status":"text"}"#;
        let message: LiveMessage = serde_json::from_slice(old).unwrap();
        assert!(message.sender_username.is_none());
        assert_eq!(serde_json::to_vec(&message).unwrap(), old);
        let mut data = window(2);
        data.rows[0].sender_username = Some("owned-sender-账号".into());
        data.rows[1] = message;
        let view = LiveReadView::new(&binding, data).unwrap();
        assert!(!view.ciphertext.windows(12).any(|b| b == b"owned-sender"));
        let page = view.page(&binding, None, 60).unwrap();
        assert_eq!(
            page.rows[0].sender_username.as_deref(),
            Some("owned-sender-账号")
        );
        assert!(page.rows[1].sender_username.is_none());
        assert!(view.page(&"b".repeat(64), None, 60).is_err());
        for invalid in [
            "".to_owned(),
            "x".repeat(257),
            "bad\0id".into(),
            "bad\nid".into(),
        ] {
            let mut data = window(1);
            data.rows[0].sender_username = Some(invalid);
            assert!(LiveReadView::new(&binding, data).is_err());
        }
    }

    #[test]
    fn source_continuation_is_separate_from_page_cursor_and_expires() {
        let binding = "a".repeat(64);
        let mut view = LiveReadView::new(&binding, window(125)).unwrap();
        let first = view.page(&binding, None, 100).unwrap();
        assert!(first.continuation.is_none());
        assert!(
            view.continuation_anchor(&binding, first.older.as_ref().unwrap())
                .is_err()
        );
        let last = view
            .page(
                &binding,
                Some((first.older.as_ref().unwrap(), LiveDirection::Older)),
                100,
            )
            .unwrap();
        let token = last.continuation.unwrap();
        assert!(
            view.page(&binding, Some((&token, LiveDirection::Older)), 100)
                .is_err()
        );
        assert_eq!(
            view.continuation_anchor(&binding, &token).unwrap().key(),
            Some((1700000000, 1))
        );
        assert!(view.continuation_anchor(&"b".repeat(64), &token).is_err());
        let other = LiveReadView::new(&binding, window(125)).unwrap();
        assert!(other.continuation_anchor(&binding, &token).is_err());
        view.created = Instant::now() - Duration::from_secs(121);
        assert!(view.continuation_anchor(&binding, &token).is_err());
    }
    #[test]
    fn same_second_window_pages_exactly_and_reverses_without_source_locks() {
        let binding = "a".repeat(64);
        let view = LiveReadView::new(&binding, window(1205)).unwrap();
        assert!(
            !view
                .ciphertext
                .windows(18)
                .any(|b| b == b"owned-view-message")
        );
        let first = view.page(&binding, None, 60).unwrap();
        let second = view
            .page(
                &binding,
                Some((first.older.as_ref().unwrap(), LiveDirection::Older)),
                60,
            )
            .unwrap();
        let back = view
            .page(
                &binding,
                Some((second.newer.as_ref().unwrap(), LiveDirection::Newer)),
                60,
            )
            .unwrap();
        assert_eq!(
            first.rows.iter().map(|r| &r.local_id).collect::<Vec<_>>(),
            back.rows.iter().map(|r| &r.local_id).collect::<Vec<_>>()
        );
        let mut page = first;
        let mut ids = Vec::new();
        loop {
            ids.extend(
                page.rows
                    .iter()
                    .map(|r| r.local_id.parse::<usize>().unwrap()),
            );
            match page.older {
                Some(c) => {
                    page = view
                        .page(&binding, Some((&c, LiveDirection::Older)), 60)
                        .unwrap()
                }
                None => break,
            }
        }
        assert_eq!(ids, (1..=1205).rev().collect::<Vec<_>>());
    }
    #[test]
    fn rejects_tampering_direction_context_expiry_and_ciphertext_corruption() {
        let binding = "a".repeat(64);
        let mut view = LiveReadView::new(&binding, window(200)).unwrap();
        let first = view.page(&binding, None, 60).unwrap();
        let cursor = first.older.unwrap();
        assert!(
            view.page(&binding, Some((&cursor, LiveDirection::Newer)), 60)
                .is_err()
        );
        assert!(view.page(&"b".repeat(64), None, 60).is_err());
        let other = LiveReadView::new(&binding, window(200)).unwrap();
        assert!(
            other
                .page(&binding, Some((&cursor, LiveDirection::Older)), 60)
                .is_err()
        );
        let mut tampered = cursor.into_bytes();
        tampered[30] = if tampered[30] == b'0' { b'1' } else { b'0' };
        assert!(
            view.page(
                &binding,
                Some((
                    std::str::from_utf8(&tampered).unwrap(),
                    LiveDirection::Older
                )),
                60
            )
            .is_err()
        );
        view.ciphertext[0] ^= 1;
        assert!(view.page(&binding, None, 60).is_err());
        view.ciphertext[0] ^= 1;
        view.created = Instant::now() - Duration::from_secs(121);
        assert!(view.page(&binding, None, 60).is_err());
    }
    #[test]
    fn rejects_unordered_oversized_and_invalid_window() {
        let binding = "a".repeat(64);
        let mut bad = window(10);
        bad.rows.swap(0, 1);
        assert!(LiveReadView::new(&binding, bad).is_err());
        assert!(LiveReadView::new(&binding, window(LIVE_WINDOW_MAX_ROWS + 1)).is_err());
        let mut bad = window(10);
        bad.rows[0].text = Some("x".repeat(16385));
        assert!(LiveReadView::new(&binding, bad).is_err());
        let view = LiveReadView::new(&binding, window(10)).unwrap();
        assert!(view.page(&binding, None, 101).is_err());
    }
    #[test]
    fn contacts_cursor_is_private_bound_to_view_source_and_distinct_from_message_cursor() {
        let binding = "a".repeat(64);
        let mut view = LiveReadView::new(&binding, window(125)).unwrap();
        let value = LiveContactContinuation {
            search_query: None,
            source_fingerprint: "c".repeat(64),
            anchor: LiveContactAnchor {
                row_id: "9007199254740993".into(),
                username: "owned-contact".into(),
            },
        };
        let token = view.seal_contacts(&binding, &view.id, &value).unwrap();
        assert!(!String::from_utf8_lossy(&hex::decode(&token).unwrap()).contains("owned-contact"));
        assert_eq!(
            view.open_contacts(&binding, &view.id, &token)
                .unwrap()
                .anchor
                .row_id,
            value.anchor.row_id
        );
        assert!(view.open_contacts(&binding, "other", &token).is_err());
        assert!(
            view.open_contacts(&"b".repeat(64), &view.id, &token)
                .is_err()
        );
        assert!(
            view.page(&binding, Some((&token, LiveDirection::Older)), 60)
                .is_err()
        );
        let message = view.page(&binding, None, 60).unwrap().older.unwrap();
        assert!(view.open_contacts(&binding, &view.id, &message).is_err());
        let other = LiveReadView::new(&binding, window(125)).unwrap();
        assert!(other.open_contacts(&binding, &other.id, &token).is_err());
        view.created = Instant::now() - Duration::from_secs(121);
        assert!(view.open_contacts(&binding, &view.id, &token).is_err());
    }
    #[test]
    fn sessions_cursor_is_private_bound_to_view_source_and_distinct_from_message_cursor() {
        let binding = "a".repeat(64);
        let mut view = LiveReadView::new(&binding, window(125)).unwrap();
        let value = LiveSessionContinuation {
            search_query: None,
            source_fingerprint: "c".repeat(64),
            anchor: LiveSessionAnchor {
                sort_timestamp: 20,
                row_id: "9007199254740993".into(),
                username: "owned-session".into(),
            },
        };
        let token = view.seal_sessions(&binding, &view.id, &value).unwrap();
        assert!(!String::from_utf8_lossy(&hex::decode(&token).unwrap()).contains("owned-session"));
        assert_eq!(
            view.open_sessions(&binding, &view.id, &token)
                .unwrap()
                .anchor
                .row_id,
            value.anchor.row_id
        );
        assert!(view.open_sessions(&binding, "other", &token).is_err());
        assert!(
            view.open_sessions(&"b".repeat(64), &view.id, &token)
                .is_err()
        );
        assert!(
            view.page(&binding, Some((&token, LiveDirection::Older)), 60)
                .is_err()
        );
        let message = view.page(&binding, None, 60).unwrap().older.unwrap();
        assert!(view.open_sessions(&binding, &view.id, &message).is_err());
        assert!(view.open_contacts(&binding, &view.id, &token).is_err());
        let other = LiveReadView::new(&binding, window(125)).unwrap();
        assert!(other.open_sessions(&binding, &other.id, &token).is_err());
        view.created = Instant::now() - Duration::from_secs(121);
        assert!(view.open_sessions(&binding, &view.id, &token).is_err());
    }
    #[test]
    fn moments_cursor_is_private_bound_to_view_source_and_distinct_from_message_cursor() {
        let binding = "a".repeat(64);
        let mut view = LiveReadView::new(&binding, window(125)).unwrap();
        use werc_domain::moments::LiveMomentAnchor;
        let value = werc_domain::moments::LiveMomentContinuation {
            source_fingerprint: "c".repeat(64),
            anchor: LiveMomentAnchor {
                id: u64::MAX.to_string(),
                row_id: "9007199254740993".into(),
                username: "owned-moment".into(),
            },
        };
        let expected_row = value.anchor.row_id.clone();
        let value = crate::live_search::SearchTicket::Moments {
            after: value,
            query: Default::default(),
        };
        let token = view.seal_search(&binding, &view.id, &value).unwrap();
        assert!(!String::from_utf8_lossy(&hex::decode(&token).unwrap()).contains("owned-moment"));
        match view.open_search(&binding, &view.id, &token).unwrap() {
            crate::live_search::SearchTicket::Moments { after, .. } => {
                assert_eq!(after.anchor.row_id, expected_row)
            }
            _ => panic!("wrong cursor purpose"),
        }
        assert!(view.open_search(&binding, "other", &token).is_err());
        assert!(view.open_search(&"b".repeat(64), &view.id, &token).is_err());
        assert!(
            view.page(&binding, Some((&token, LiveDirection::Older)), 60)
                .is_err()
        );
        let message = view.page(&binding, None, 60).unwrap().older.unwrap();
        assert!(view.open_search(&binding, &view.id, &message).is_err());
        assert!(view.open_contacts(&binding, &view.id, &token).is_err());
        let other = LiveReadView::new(&binding, window(125)).unwrap();
        assert!(other.open_search(&binding, &other.id, &token).is_err());
        view.created = Instant::now() - Duration::from_secs(121);
        assert!(view.open_search(&binding, &view.id, &token).is_err());
    }

    #[test]
    fn group_cursor_binds_identity_purpose_and_expiration() {
        use werc_domain::group::*;
        let binding = "a".repeat(64);
        let mut view = LiveReadView::new(&binding, window(125)).unwrap();
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
