//! Contact activity over encrypted saved messages. No sending or bot policy.
use crate::{Application, capture_tools::CaptureFilter, source_capture::CaptureRow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use werc_domain::{
    AppError, AppResult,
    live::{is_group_account, is_private_account, valid_live_username},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineQuery {
    #[serde(default)]
    pub mode: TimelineMode,
    #[serde(default)]
    pub text: String,
    pub username: Option<String>,
    pub from: Option<i64>,
    pub until: Option<i64>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineMode {
    #[default]
    All,
    Private,
    WaitingMe,
    WaitingThem,
    Mentions,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineCursor {
    pub timestamp: i64,
    pub page: u64,
    pub row: usize,
    pub snapshot: String,
}
#[derive(Serialize)]
pub struct TimelineItem {
    pub page: u64,
    pub row: usize,
    pub timestamp: i64,
    pub username: String,
    pub sender: Option<String>,
    pub message_type: String,
    pub text: Option<String>,
    pub local_status: String,
    pub direction: String,
    pub mentions_me: bool,
    pub mentions_all: bool,
    pub pending: Option<String>,
}
#[derive(Default, Serialize)]
pub struct TimelineSummary {
    pub sent_contacts: usize,
    pub received_contacts: usize,
    pub waiting_me: usize,
    pub waiting_them: usize,
    pub mentions: u64,
    pub mention_groups: usize,
    pub group_rows_without_metadata: u64,
}
#[derive(Serialize)]
pub struct ContactTimeline {
    pub data: Vec<TimelineItem>,
    pub next: Option<TimelineCursor>,
    pub summary: TimelineSummary,
    pub complete_selected_sources: bool,
    pub scanned: u64,
    pub duplicates_skipped: u64,
}
type Key = (i64, u64, usize);
fn key(item: &TimelineItem) -> Key {
    (item.timestamp, item.page, item.row)
}
fn direction(row: &CaptureRow, login: &str) -> &'static str {
    match row.message.sender_username.as_deref() {
        Some(s) if !login.is_empty() && s == login => "outgoing",
        Some(s)
            if !login.is_empty()
                && !s.is_empty()
                && (is_group_account(&row.username) || s == row.username) =>
        {
            "incoming"
        }
        _ => "unknown",
    }
}
impl Application {
    fn timeline_snapshot(
        &self,
        account: &str,
        id: &str,
        query: &TimelineQuery,
        request_id: &str,
    ) -> AppResult<String> {
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let content = self.capture_revision_snapshot(account, id, || lease.check())?;
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&(content, query)).map_err(|_| AppError::internal())?);
        Ok(hex::encode(digest.finalize()))
    }
    pub fn capture_timeline_local(
        &self,
        account: &str,
        id: &str,
        query: &TimelineQuery,
        after: Option<&TimelineCursor>,
        request_id: &str,
    ) -> AppResult<ContactTimeline> {
        if query.text.len() > 512
            || query.text.contains('\0')
            || query
                .username
                .as_deref()
                .is_some_and(|s| !valid_live_username(s))
            || matches!((query.from,query.until),(Some(a),Some(b)) if a>=b)
            || after.is_some_and(|a| {
                a.snapshot.len() != 64 || !a.snapshot.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "时间线筛选或续读位置无效",
            ));
        }
        let snapshot = self.timeline_snapshot(account, id, query, request_id)?;
        if after.is_some_and(|a| a.snapshot != snapshot) {
            return Err(AppError::new(
                "CURSOR_STALE",
                "资料或筛选已变化，请重新查看时间线",
            ));
        }
        let login = self.capture_login_local(account, id)?;
        let filter = CaptureFilter {
            from: query.from,
            until: query.until,
            conversations: query.username.iter().cloned().collect(),
            deduplicate: true,
            ..Default::default()
        };
        let mut latest = BTreeMap::<String, (Key, String)>::new();
        let mut sent = BTreeSet::new();
        let mut received = BTreeSet::new();
        let mut groups = BTreeSet::new();
        let mut summary = TimelineSummary::default();
        let mut scanned = 0;
        let mut candidates = BTreeMap::<Key, TimelineItem>::new();
        let needle = query.text.to_lowercase();
        // Waiting views need the newest non-system observation from the whole
        // selected period, before applying content search or a page boundary.
        let (complete, _, duplicates) = self.visit_capture_positions_local(
            account,
            id,
            &filter,
            request_id,
            |page, index, row| {
                scanned += 1;
                let m = &row.message;
                let dir = direction(&row, &login);
                let group = is_group_account(&row.username);
                let private = is_private_account(&row.username);
                let system = matches!(
                    m.message_type.parse::<u64>().unwrap_or(0) & 0xffffffff,
                    10000 | 10002
                );
                if private && !system {
                    match dir {
                        "outgoing" => {
                            sent.insert(row.username.clone());
                        }
                        "incoming" => {
                            received.insert(row.username.clone());
                        }
                        _ => {}
                    }
                    let entry = latest
                        .entry(row.username.clone())
                        .or_insert(((i64::MIN, 0, 0), "unknown".into()));
                    if m.timestamp > entry.0.0 {
                        *entry = ((m.timestamp, page, index), dir.into());
                    } else if m.timestamp == entry.0.0 {
                        entry.0 = entry.0.max((m.timestamp, page, index));
                        if entry.1 != dir {
                            entry.1 = "unknown".into();
                        }
                    }
                }
                let incoming_group = group && dir == "incoming";
                let mentions_me = incoming_group
                    && !login.is_empty()
                    && m.at_user_list
                        .as_ref()
                        .is_some_and(|u| u.iter().any(|s| s == &login));
                let mentions_all = incoming_group
                    && m.at_user_list
                        .as_ref()
                        .is_some_and(|u| u.iter().any(|s| s == "notify@all"));
                if incoming_group && m.at_user_list.is_none() {
                    summary.group_rows_without_metadata += 1;
                }
                if mentions_me || mentions_all {
                    summary.mentions += 1;
                    groups.insert(row.username.clone());
                }
                let included = match query.mode {
                    TimelineMode::All => true,
                    TimelineMode::Private => private,
                    TimelineMode::Mentions => mentions_me || mentions_all,
                    TimelineMode::WaitingMe | TimelineMode::WaitingThem => false,
                };
                if !included
                    || (!needle.is_empty()
                        && !m
                            .text
                            .as_deref()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(&needle))
                {
                    return Ok(());
                }
                let item = TimelineItem {
                    page,
                    row: index,
                    timestamp: m.timestamp,
                    username: row.username.clone(),
                    sender: m.sender_username.clone(),
                    message_type: m.message_type.clone(),
                    text: if m.message_type == "1" {
                        m.text.as_ref().map(|s| s.chars().take(240).collect())
                    } else {
                        None
                    },
                    local_status: m.content_status.clone(),
                    direction: dir.into(),
                    mentions_me,
                    mentions_all,
                    pending: None,
                };
                keep(&mut candidates, item, after);
                Ok(())
            },
        )?;
        summary.sent_contacts = sent.len();
        summary.received_contacts = received.len();
        summary.mention_groups = groups.len();
        for (_, dir) in latest.values() {
            match dir.as_str() {
                "incoming" => summary.waiting_me += 1,
                "outgoing" => summary.waiting_them += 1,
                _ => {}
            }
        }
        if matches!(
            query.mode,
            TimelineMode::WaitingMe | TimelineMode::WaitingThem
        ) {
            self.visit_capture_positions_local(
                account,
                id,
                &filter,
                request_id,
                |page, index, row| {
                    let Some((last, dir)) = latest.get(&row.username) else {
                        return Ok(());
                    };
                    let expected = if query.mode == TimelineMode::WaitingMe {
                        "incoming"
                    } else {
                        "outgoing"
                    };
                    let m = &row.message;
                    if *last != (m.timestamp, page, index)
                        || dir != expected
                        || (!needle.is_empty()
                            && !m
                                .text
                                .as_deref()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&needle))
                    {
                        return Ok(());
                    }
                    keep(
                        &mut candidates,
                        TimelineItem {
                            page,
                            row: index,
                            timestamp: m.timestamp,
                            username: row.username.clone(),
                            sender: m.sender_username.clone(),
                            message_type: m.message_type.clone(),
                            text: if m.message_type == "1" {
                                m.text.as_ref().map(|s| s.chars().take(240).collect())
                            } else {
                                None
                            },
                            local_status: m.content_status.clone(),
                            direction: dir.clone(),
                            mentions_me: false,
                            mentions_all: false,
                            pending: Some(
                                if expected == "incoming" {
                                    "waiting_me"
                                } else {
                                    "waiting_them"
                                }
                                .into(),
                            ),
                        },
                        after,
                    );
                    Ok(())
                },
            )?;
        } else {
            for item in candidates.values_mut() {
                if let Some((last, dir)) = latest
                    .get(&item.username)
                    .filter(|(last, _)| *last == key(item))
                {
                    let _ = last;
                    item.pending = match dir.as_str() {
                        "incoming" => Some("waiting_me".into()),
                        "outgoing" => Some("waiting_them".into()),
                        _ => None,
                    };
                }
            }
        }
        if self.timeline_snapshot(account, id, query, request_id)? != snapshot {
            return Err(AppError::new(
                "DOCUMENT_CONFLICT",
                "分析期间资料已变化，请重新读取",
            ));
        }
        let has_more = candidates.len() > 100;
        if has_more {
            candidates.pop_first();
        }
        let data: Vec<_> = candidates.into_values().rev().collect();
        let next = has_more.then(|| {
            let item = data.last().expect("bounded page");
            TimelineCursor {
                timestamp: item.timestamp,
                page: item.page,
                row: item.row,
                snapshot,
            }
        });
        Ok(ContactTimeline {
            data,
            next,
            summary,
            complete_selected_sources: complete,
            scanned,
            duplicates_skipped: duplicates,
        })
    }
}
fn keep(
    items: &mut BTreeMap<Key, TimelineItem>,
    item: TimelineItem,
    after: Option<&TimelineCursor>,
) {
    let k = key(&item);
    if after.is_some_and(|a| k >= (a.timestamp, a.page, a.row)) {
        return;
    }
    items.insert(k, item);
    if items.len() > 101 {
        items.pop_first();
    }
}
