mod browse;
pub(crate) use browse::QueryLease;
pub(crate) use browse::QueryJob;
mod projection;
use crate::Application;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use werc_domain::{AppError, AppResult, Principal, history::*};

type MessageKey = (i64, i64, String);
const QUERY_RETAINED_BYTES: usize = 32 * 1024 * 1024;
fn retained_bytes(value: &impl Serialize) -> AppResult<usize> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("query size"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value).map_err(|_| AppError::internal())?;
    counter
        .0
        .checked_mul(2)
        .and_then(|n| n.checked_add(256))
        .ok_or_else(AppError::internal)
}
#[derive(Default)]
struct QueryBudget(usize);
impl QueryBudget {
    fn replace(&mut self, old: usize, new: usize) -> AppResult<()> {
        self.0 = self
            .0
            .checked_add(new)
            .and_then(|n| n.checked_sub(old))
            .ok_or_else(AppError::internal)?;
        if self.0 > QUERY_RETAINED_BYTES {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "查询结果超过32MiB保留预算，请缩小范围或页大小",
            ));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    fingerprint: String,
    after: MessageKey,
}
fn key(message: &HistoryMessage) -> MessageKey {
    (message.timestamp, message.sequence, message.id.clone())
}
fn query_valid(q: &HistoryQuery) -> AppResult<()> {
    if !(1..=200).contains(&q.limit)
        || q.search.as_ref().is_some_and(|s| s.chars().count() > 200)
        || q.conversation_id.as_ref().is_some_and(|s| s.len() > 512)
        || q.sender_id.as_ref().is_some_and(|s| s.len() > 512)
        || q.message_type.is_some_and(|v| v < 0)
        || q.from.is_some_and(|v| v < 0)
        || q.until.is_some_and(|v| v < 0)
        || matches!((q.from,q.until),(Some(a),Some(b)) if a>=b)
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "查询范围、关键词或页大小不合法",
        ));
    }
    Ok(())
}
fn matches(message: &HistoryMessage, q: &HistoryQuery, search: &Option<String>) -> bool {
    q.conversation_id
        .as_ref()
        .is_none_or(|v| v == &message.conversation_id)
        && q.sender_id
            .as_ref()
            .is_none_or(|v| message.sender_id.as_ref() == Some(v))
        && q.message_type.is_none_or(|v| message.message_type == v)
        && q.from.is_none_or(|v| message.timestamp >= v)
        && q.until.is_none_or(|v| message.timestamp < v)
        && search.as_ref().is_none_or(|s| {
            message
                .text
                .as_ref()
                .is_some_and(|t| t.to_lowercase().contains(s))
        })
}
impl Application {
    pub fn history_sources_local(&self, account: &str) -> AppResult<Vec<HistorySource>> {
        self.ensure_account(account)?;
        self.repository.history_sources(account)
    }
    pub fn history_sources(&self, p: &Principal, account: &str) -> AppResult<Vec<HistorySource>> {
        p.require("sources:read")?;
        p.require_account(account)?;
        self.history_sources_local(account)
    }
    pub fn remove_history_local(&self, account: &str, id: &str) -> AppResult<()> {
        self.ensure_account(account)?;
        self.repository.remove_history(account, id)
    }
    pub fn history_contacts_local(&self, account: &str) -> AppResult<Vec<HistoryContact>> {
        let mut contacts = BTreeMap::new();
        let mut budget = QueryBudget::default();
        self.visit_history(account, &mut |_, bundle| {
            for contact in bundle.contacts {
                let old = contacts
                    .get(&contact.username)
                    .map(retained_bytes)
                    .transpose()?
                    .unwrap_or(0);
                budget.replace(old, retained_bytes(&contact)?)?;
                contacts.insert(contact.username.clone(), contact);
            }
            Ok(())
        })?;
        Ok(contacts.into_values().collect())
    }
    pub fn history_contacts(&self, p: &Principal, account: &str) -> AppResult<Vec<HistoryContact>> {
        p.require("contacts:read")?;
        p.require_account(account)?;
        self.history_contacts_local(account)
    }
    pub fn history_sessions_local(&self, account: &str) -> AppResult<Vec<HistorySession>> {
        let mut sessions: BTreeMap<String, HistorySession> = BTreeMap::new();
        let mut budget = QueryBudget::default();
        self.visit_history(account, &mut |_, bundle| {
            for mut session in bundle.sessions {
                if let Some(previous) = sessions.get(&session.username)
                    && previous.timestamp > session.timestamp
                {
                    session.timestamp = previous.timestamp;
                    session.summary = previous.summary.clone();
                }
                let old = sessions
                    .get(&session.username)
                    .map(retained_bytes)
                    .transpose()?
                    .unwrap_or(0);
                budget.replace(old, retained_bytes(&session)?)?;
                sessions.insert(session.username.clone(), session);
            }
            for message in bundle.messages {
                let old = sessions
                    .get(&message.conversation_id)
                    .map(retained_bytes)
                    .transpose()?
                    .unwrap_or(0);
                let session = sessions
                    .entry(message.conversation_id.clone())
                    .or_insert_with(|| HistorySession {
                        username: message.conversation_id,
                        summary: None,
                        timestamp: 0,
                        unread_count: None,
                    });
                if message.timestamp > session.timestamp {
                    session.timestamp = message.timestamp;
                    session.summary = message.text;
                }
                budget.replace(old, retained_bytes(session)?)?;
            }
            Ok(())
        })?;
        let mut result: Vec<_> = sessions.into_values().collect();
        result.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then(a.username.cmp(&b.username))
        });
        Ok(result)
    }
    pub fn history_sessions(&self, p: &Principal, account: &str) -> AppResult<Vec<HistorySession>> {
        p.require("sessions:read")?;
        p.require_account(account)?;
        self.history_sessions_local(account)
    }
    fn sources_for_query(&self, account: &str) -> AppResult<Vec<HistorySource>> {
        let mut sources = self.history_sources_local(account)?;
        sources.sort_by(|a, b| a.imported_at.cmp(&b.imported_at).then(a.id.cmp(&b.id)));
        if sources.is_empty() {
            return Err(AppError::new(
                "CAPABILITY_NOT_IMPLEMENTED",
                "尚无已导入的可读来源；没有读取实时微信",
            ));
        }
        Ok(sources)
    }
    fn check_query_sources(&self, account: &str, sources: &[HistorySource]) -> AppResult<()> {
        let mut current = self.history_sources_local(account)?;
        current.sort_by(|a, b| a.imported_at.cmp(&b.imported_at).then(a.id.cmp(&b.id)));
        if serde_json::to_vec(&current).map_err(|_| AppError::internal())?
            != serde_json::to_vec(sources).map_err(|_| AppError::internal())?
        {
            return Err(AppError::new(
                "CURSOR_STALE",
                "查询期间来源已变化，请重新查询",
            ));
        }
        Ok(())
    }
    fn scan_messages(
        &self,
        account: &str,
        sources: &[HistorySource],
        filter: &dyn Fn(&HistoryMessage) -> bool,
        visitor: &mut dyn FnMut(HistoryMessage) -> AppResult<()>,
    ) -> AppResult<()> {
        self.scan_messages_controlled(account, sources, filter, &|| Ok(()), visitor)
    }
    fn scan_messages_controlled(
        &self,
        account: &str,
        sources: &[HistorySource],
        filter: &dyn Fn(&HistoryMessage) -> bool,
        check: &dyn Fn() -> AppResult<()>,
        visitor: &mut dyn FnMut(HistoryMessage) -> AppResult<()>,
    ) -> AppResult<()> {
        check()?;
        let mut legacy_current = BTreeMap::new();
        let index_key = self.history_index_secret(account)?;
        for source in sources {
            check()?;
            if self
                .repository
                .history_payload(account, &source.id)?
                .version
                == 1
            {
                self.visit_history_source_controlled(source, check, &mut |bundle| {
                    check()?;
                    for message in bundle.messages {
                        legacy_current.insert(message.id, source.id.clone());
                    }
                    Ok(())
                })?;
            }
        }
        for source in sources {
            check()?;
            let legacy = self
                .repository
                .history_payload(account, &source.id)?
                .version
                == 1;
            self.visit_history_source_controlled(source, check, &mut |bundle| {
                check()?;
                let mut selected = bundle.messages.into_iter().filter(|m| filter(m));
                loop {
                    let batch: Vec<_> = selected.by_ref().take(HISTORY_BATCH_ROWS).collect();
                    if batch.is_empty() {
                        break;
                    }
                    let ids: Vec<_> = batch.iter().map(|m| m.id.clone()).collect();
                    let states = self.repository.history_message_states(account, &ids)?;
                    for message in batch {
                        if legacy && legacy_current.get(&message.id) != Some(&source.id) {
                            continue;
                        }
                        let state = states.get(&message.id);
                        if state.is_some_and(|s| s.source_id != source.id) {
                            continue;
                        }
                        if !legacy {
                            let digest =
                                crate::history_stream::message_digest(&index_key, &message)?;
                            if state.is_none_or(|s| s.digest != digest) {
                                continue;
                            }
                        }
                        visitor(message)?;
                    }
                }
                Ok(())
            })?;
        }
        self.check_query_sources(account, sources)?;
        Ok(())
    }
    pub fn history_messages_local(
        &self,
        account: &str,
        q: &HistoryQuery,
    ) -> AppResult<MessagePage> {
        query_valid(q)?;
        let sources = self.sources_for_query(account)?;
        let digest = serde_json::to_vec(&(
            account,
            &sources,
            &q.conversation_id,
            &q.sender_id,
            q.message_type,
            &q.search,
            q.from,
            q.until,
        ))
        .map_err(|_| AppError::internal())?;
        let fingerprint = hex::encode(Sha256::digest(&digest));
        let after = if let Some(cursor) = &q.cursor {
            if cursor.len() > 2048 {
                return Err(AppError::new("INVALID_ARGUMENT", "游标无效"));
            }
            let bytes =
                hex::decode(cursor).map_err(|_| AppError::new("INVALID_ARGUMENT", "游标无效"))?;
            let cursor: Cursor = serde_json::from_slice(&bytes)
                .map_err(|_| AppError::new("INVALID_ARGUMENT", "游标无效"))?;
            if cursor.fingerprint != fingerprint {
                return Err(AppError::new(
                    "CURSOR_STALE",
                    "来源或查询条件已变化，请从第一页重新查询",
                ));
            }
            Some(cursor.after)
        } else {
            None
        };
        let search = q.search.as_ref().map(|s| s.to_lowercase());
        let mut page = BTreeMap::new();
        let mut budget = QueryBudget::default();
        let filter = |message: &HistoryMessage| {
            after.as_ref().is_none_or(|v| key(message) > *v) && matches(message, q, &search)
        };
        self.scan_messages(account, &sources, &filter, &mut |message| {
            let order = key(&message);
            let new = retained_bytes(&message)?;
            let old = page
                .insert(order, message)
                .as_ref()
                .map(retained_bytes)
                .transpose()?
                .unwrap_or(0);
            let removed = if page.len() > q.limit as usize + 1 {
                page.pop_last()
                    .map(|(_, m)| retained_bytes(&m))
                    .transpose()?
                    .unwrap_or(0)
            } else {
                0
            };
            budget.replace(old + removed, new)?;
            Ok(())
        })?;
        let more = page.len() > q.limit as usize;
        if more {
            page.pop_last();
        }
        let next_cursor = if more {
            Some(hex::encode(
                serde_json::to_vec(&Cursor {
                    fingerprint,
                    after: page
                        .last_key_value()
                        .ok_or_else(AppError::internal)?
                        .0
                        .clone(),
                })
                .map_err(|_| AppError::internal())?,
            ))
        } else {
            None
        };
        Ok(MessagePage {
            data: page.into_values().collect(),
            next_cursor,
            source_ids: sources.into_iter().map(|s| s.id).collect(),
            data_mode: "imported_snapshot".into(),
        })
    }
    pub fn history_messages(
        &self,
        p: &Principal,
        account: &str,
        q: &HistoryQuery,
    ) -> AppResult<MessagePage> {
        p.require("messages:read")?;
        p.require_account(account)?;
        self.history_messages_local(account, q)
    }
    pub fn export_history_local(
        &self,
        account: &str,
        q: &HistoryQuery,
        format: &str,
    ) -> AppResult<String> {
        query_valid(q)?;
        let sources = self.sources_for_query(account)?;
        let search = q.search.as_ref().map(|s| s.to_lowercase());
        let mut messages = BTreeMap::new();
        let mut bytes = 0usize;
        self.scan_messages(
            account,
            &sources,
            &|message| matches(message, q, &search),
            &mut |message| {
                {
                    bytes = bytes
                        .checked_add(
                            serde_json::to_vec(&message)
                                .map_err(|_| AppError::internal())?
                                .len(),
                        )
                        .ok_or_else(AppError::internal)?;
                    if bytes > 32 * 1024 * 1024 {
                        return Err(AppError::new(
                            "QUOTA_EXCEEDED",
                            "本次导出超过32MiB编码预算，请缩小时间范围",
                        ));
                    }
                    messages.insert(key(&message), message);
                }
                Ok(())
            },
        )?;
        crate::history_export::export(
            account,
            &sources,
            &messages.into_values().collect::<Vec<_>>(),
            format,
        )
    }
    pub fn export_history(
        &self,
        p: &Principal,
        account: &str,
        q: &HistoryQuery,
        format: &str,
    ) -> AppResult<String> {
        p.require("exports:create")?;
        p.require("messages:read")?;
        p.require_account(account)?;
        self.export_history_local(account, q, format)
    }
}
