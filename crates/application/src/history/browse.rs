use super::*;
use chrono::Utc;
use hmac::{Hmac, Mac};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;
use werc_domain::browse::*;

pub(crate) struct QueryLease<'a> {
    app: &'a Application,
    id: String,
    pub(crate) cancel: Arc<AtomicBool>,
    started: Instant,
    timeout_seconds: u64,
    pub(super) batches: std::cell::Cell<u64>,
    pub(super) projection_built: std::cell::Cell<bool>,
}
pub(crate) struct QueryJob {
    pub(crate) cancel: Arc<AtomicBool>,
    source: bool,
}
impl QueryLease<'_> {
    pub(crate) fn for_local_archive(mut self) -> Self {
        self.timeout_seconds = 3600;
        self
    }
    fn metrics(&self) -> QueryMetrics {
        QueryMetrics {
            authenticated_batches: self.batches.get(),
            elapsed_ms: self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            projection_built: self.projection_built.get(),
        }
    }
    pub(crate) fn check(&self) -> AppResult<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(AppError::new("QUERY_CANCELED", "查询已取消"));
        }
        if self.started.elapsed().as_secs() >= self.timeout_seconds {
            return Err(AppError::new(
                "QUERY_TIMEOUT",
                "资料读取超时；可缩小范围或稍后继续",
            ));
        }
        Ok(())
    }
}
impl Drop for QueryLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut jobs) = self.app.browse_jobs.lock() {
            jobs.remove(&self.id);
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowseCursor {
    version: u32,
    fingerprint: String,
    purpose: String,
    key: MessageKey,
}
fn seal(cursor: BrowseCursor, secret: &[u8]) -> AppResult<String> {
    let bytes = serde_json::to_vec(&cursor).map_err(|_| AppError::internal())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| AppError::internal())?;
    mac.update(&bytes);
    Ok(format!(
        "{}.{}",
        hex::encode(&bytes),
        hex::encode(mac.finalize().into_bytes())
    ))
}
fn unseal(text: &str, secret: &[u8], fingerprint: &str, purpose: &str) -> AppResult<MessageKey> {
    let invalid = || {
        AppError::new(
            "CURSOR_STALE",
            "游标已失效或与来源、方向、条件不符，请重新定位",
        )
    };
    if text.len() > 4096 {
        return Err(invalid());
    }
    let (payload, signature) = text.split_once('.').ok_or_else(invalid)?;
    let bytes = hex::decode(payload).map_err(|_| invalid())?;
    let signature = hex::decode(signature).map_err(|_| invalid())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| AppError::internal())?;
    mac.update(&bytes);
    mac.verify_slice(&signature).map_err(|_| invalid())?;
    let cursor: BrowseCursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if cursor.version != 1 || cursor.fingerprint != fingerprint || cursor.purpose != purpose {
        return Err(invalid());
    }
    Ok(cursor.key)
}
fn direction_name(direction: BrowseDirection) -> &'static str {
    match direction {
        BrowseDirection::Latest => "latest",
        BrowseDirection::Older => "older",
        BrowseDirection::Newer => "newer",
        BrowseDirection::Around => "around",
    }
}

impl Application {
    pub(crate) fn browse_lease(&self, id: &str) -> AppResult<QueryLease<'_>> {
        self.query_lease(id, false)
    }
    pub(crate) fn source_lease(&self, id: &str) -> AppResult<QueryLease<'_>> {
        self.query_lease(id, true)
    }
    fn query_lease(&self, id: &str, source: bool) -> AppResult<QueryLease<'_>> {
        if !(16..=80).contains(&id.len())
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效"));
        }
        let mut canceled = self
            .browse_canceled
            .lock()
            .map_err(|_| AppError::internal())?;
        if let Some(index) = canceled.iter().position(|value| value == id) {
            canceled.remove(index);
            return Err(AppError::new("QUERY_CANCELED", "查询在启动前已取消"));
        }
        let mut jobs = self.browse_jobs.lock().map_err(|_| AppError::internal())?;
        // Online readers use isolated workers. Allow a CPU-scaled budget while
        // keeping the heavier archive/projection budget independent.
        let limit = if source {
            std::thread::available_parallelism().map(usize::from).unwrap_or(2).clamp(2, 8)
        } else { 2 };
        if jobs.contains_key(id) || jobs.values().filter(|job| job.source == source).count() >= limit {
            return Err(AppError::new("BUSY", if source { "在线查询已达到本机并发额度，请稍后重试" } else { "已有快照查询正在执行或取消中" }));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(id.into(), QueryJob { cancel: cancel.clone(), source });
        Ok(QueryLease {
            app: self,
            id: id.into(),
            cancel,
            started: Instant::now(),
            timeout_seconds: 120,
            batches: std::cell::Cell::new(0),
            projection_built: std::cell::Cell::new(false),
        })
    }
    pub fn cancel_browse_local(&self, id: &str) -> AppResult<()> {
        if !(16..=80).contains(&id.len())
            || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(AppError::new("INVALID_ARGUMENT", "查询标识格式无效"));
        }
        // 与登记保持相同锁顺序，取消先到时也不能丢失。
        let mut canceled = self
            .browse_canceled
            .lock()
            .map_err(|_| AppError::internal())?;
        let jobs = self.browse_jobs.lock().map_err(|_| AppError::internal())?;
        if let Some(job) = jobs.get(id) {
            job.cancel.store(true, Ordering::SeqCst);
        } else if !canceled.iter().any(|value| value == id) {
            if canceled.len() >= 256 {
                canceled.pop_front();
            }
            canceled.push_back(id.into());
        }
        Ok(())
    }
    fn browse_context(
        &self,
        account: &str,
        request: &BrowseRequest,
    ) -> AppResult<(ReadContext, Vec<HistorySource>)> {
        if request.mode != ReadMode::ImportedSnapshot {
            return Err(AppError::new(
                "CAPABILITY_NOT_IMPLEMENTED",
                "此读取模式尚未装配；没有改用历史快照或自动导入",
            ));
        }
        query_valid(&request.query)?;
        if request.anchor_id.as_ref().is_some_and(|id| id.len() > 512)
            || request.anchor_timestamp.is_some_and(|v| v < 0)
            || (request.anchor_id.is_some() && request.anchor_timestamp.is_some())
        {
            return Err(AppError::new("INVALID_ARGUMENT", "消息锚点无效"));
        }
        let all = self.sources_for_query(account)?;
        let selected: Vec<_> = all
            .iter()
            .filter(|s| request.source_id.as_ref().is_none_or(|id| id == &s.id))
            .cloned()
            .collect();
        if selected.is_empty() {
            return Err(AppError::new(
                "SOURCE_NOT_FOUND",
                "快照来源不存在或已被移除",
            ));
        }
        let revision = self.current_connection_local()?.revision;
        let q = &request.query;
        let generation = hex::encode(Sha256::digest(
            serde_json::to_vec(&(
                1,
                account,
                &revision,
                &all,
                &request.source_id,
                &q.conversation_id,
                &q.sender_id,
                q.message_type,
                &q.search,
                q.from,
                q.until,
            ))
            .map_err(|_| AppError::internal())?,
        ));
        Ok((
            ReadContext {
                mode: ReadMode::ImportedSnapshot,
                account_id: account.into(),
                connection_revision: revision,
                generation,
                observed_at: Utc::now().to_rfc3339(),
                sources: selected,
                identity_status: "unverified".into(),
                query_strategy: "authenticated_snapshot_scan".into(),
                query_metrics: None,
            },
            all,
        ))
    }
    pub(super) fn check_browse_context(
        &self,
        context: &ReadContext,
        all: &[HistorySource],
        lease: &QueryLease<'_>,
    ) -> AppResult<()> {
        lease.check()?;
        self.check_query_sources(&context.account_id, all)?;
        if self.current_connection_local()?.revision != context.connection_revision {
            return Err(AppError::new(
                "CURSOR_STALE",
                "连接修订已变化，请重新查看归档",
            ));
        }
        Ok(())
    }
    fn browse_scan(
        &self,
        context: &ReadContext,
        all: &[HistorySource],
        lease: &QueryLease<'_>,
        projection: Option<(&super::projection::Projection, &str)>,
        filter: &dyn Fn(&HistoryMessage) -> bool,
        visitor: &mut dyn FnMut(HistoryMessage) -> AppResult<()>,
    ) -> AppResult<()> {
        if let Some((projection, conversation)) = projection {
            self.scan_projection(projection, (context, conversation), lease, filter, visitor)?;
            return self.check_browse_context(context, all, lease);
        }
        if context.sources.len() == all.len() {
            self.scan_messages_controlled(
                &context.account_id,
                all,
                filter,
                &|| lease.check(),
                visitor,
            )?;
        } else {
            // 单独快照展示其历史版本，不以另一个快照的“当前记录”覆盖它。
            for source in &context.sources {
                lease.check()?;
                self.visit_history_source_controlled(source, &|| lease.check(), &mut |bundle| {
                    lease.check()?;
                    for message in bundle.messages {
                        if filter(&message) {
                            visitor(message)?;
                        }
                    }
                    Ok(())
                })?;
            }
        }
        self.check_browse_context(context, all, lease)
    }
    fn browse_metadata(
        &self,
        context: &ReadContext,
        lease: &QueryLease<'_>,
        budget: &mut QueryBudget,
        projection: Option<&super::projection::Projection>,
    ) -> AppResult<(
        BTreeMap<String, HistoryContact>,
        BTreeMap<String, HistorySession>,
    )> {
        if let Some(projection) = projection {
            lease.check()?;
            budget.replace(0, retained_bytes(&projection.header.contacts)?)?;
            budget.replace(0, retained_bytes(&projection.header.sessions)?)?;
            return Ok((
                projection.header.contacts.clone(),
                projection.header.sessions.clone(),
            ));
        }
        let mut contacts = BTreeMap::new();
        let mut sessions: BTreeMap<String, HistorySession> = BTreeMap::new();
        for source in &context.sources {
            lease.check()?;
            self.visit_history_source_controlled(source, &|| lease.check(), &mut |bundle| {
                lease.check()?;
                for contact in bundle.contacts {
                    let old = contacts
                        .get(&contact.username)
                        .map(retained_bytes)
                        .transpose()?
                        .unwrap_or(0);
                    budget.replace(old, retained_bytes(&contact)?)?;
                    contacts.insert(contact.username.clone(), contact);
                }
                for session in bundle.sessions {
                    if sessions
                        .get(&session.username)
                        .is_some_and(|s| s.timestamp > session.timestamp)
                    {
                        continue;
                    }
                    let old = sessions
                        .get(&session.username)
                        .map(retained_bytes)
                        .transpose()?
                        .unwrap_or(0);
                    budget.replace(old, retained_bytes(&session)?)?;
                    sessions.insert(session.username.clone(), session);
                }
                Ok(())
            })?;
        }
        Ok((contacts, sessions))
    }
    pub fn browse_conversations_local(
        &self,
        account: &str,
        request: &BrowseRequest,
    ) -> AppResult<ConversationPage> {
        let lease = self.browse_lease(&request.request_id)?;
        let (mut context, all) = self.browse_context(account, request)?;
        let projection = self.browse_projection(&context, &all, &lease)?;
        if projection.is_some() {
            context.query_strategy = "protected_batch_projection".into();
        }
        let secret = self.history_index_secret(account)?;
        let after = request
            .query
            .cursor
            .as_ref()
            .map(|c| unseal(c, &secret, &context.generation, "conversations"))
            .transpose()?;
        let mut budget = QueryBudget::default();
        let (contacts, mut sessions) =
            self.browse_metadata(&context, &lease, &mut budget, projection.as_ref())?;
        if projection.is_none() {
            self.browse_scan(&context, &all, &lease, None, &|_| true, &mut |message| {
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
                if message.timestamp >= session.timestamp {
                    session.timestamp = message.timestamp;
                    session.summary = message.text;
                }
                budget.replace(old, retained_bytes(session)?)?;
                Ok(())
            })?;
        }
        let search = request.query.search.as_ref().map(|s| s.to_lowercase());
        let mut rows: BTreeMap<MessageKey, ConversationView> = BTreeMap::new();
        let page_budget = &mut budget;
        for session in sessions.into_values() {
            let name = contacts
                .get(&session.username)
                .and_then(|c| {
                    c.remark
                        .as_ref()
                        .filter(|s| !s.is_empty())
                        .or(c.nickname.as_ref().filter(|s| !s.is_empty()))
                })
                .cloned()
                .unwrap_or_else(|| session.username.clone());
            if search.as_ref().is_some_and(|s| {
                !name.to_lowercase().contains(s) && !session.username.to_lowercase().contains(s)
            }) {
                continue;
            }
            let order = (!session.timestamp, 0, session.username.clone());
            if after.as_ref().is_some_and(|after| order <= *after) {
                continue;
            }
            let row = ConversationView {
                id: session.username.clone(),
                display_name: name,
                identity_status: if session.username.starts_with("table:") {
                    "unresolved"
                } else {
                    "local_identifier"
                }
                .into(),
                summary: session.summary,
                timestamp: session.timestamp,
                historical_unread_count: session.unread_count.map(|n| n.to_string()),
            };
            page_budget.replace(0, retained_bytes(&row)?)?;
            rows.insert(order, row);
            if rows.len() > request.query.limit as usize + 1 {
                let (_, removed) = rows.pop_last().ok_or_else(AppError::internal)?;
                page_budget.replace(retained_bytes(&removed)?, 0)?;
            }
        }
        let more = rows.len() > request.query.limit as usize;
        if more {
            rows.pop_last();
        }
        let next_cursor = if more {
            Some(seal(
                BrowseCursor {
                    version: 1,
                    fingerprint: context.generation.clone(),
                    purpose: "conversations".into(),
                    key: rows
                        .last_key_value()
                        .ok_or_else(AppError::internal)?
                        .0
                        .clone(),
                },
                &secret,
            )?)
        } else {
            None
        };
        self.check_browse_context(&context, &all, &lease)?;
        if projection.is_some() {
            context.query_metrics = Some(lease.metrics());
        }
        Ok(ConversationPage {
            data: rows.into_values().collect(),
            next_cursor,
            context,
        })
    }
    pub fn browse_messages_local(
        &self,
        account: &str,
        request: &BrowseRequest,
    ) -> AppResult<TimelinePage> {
        let lease = self.browse_lease(&request.request_id)?;
        let (mut context, all) = self.browse_context(account, request)?;
        let q = &request.query;
        if q.conversation_id.as_ref().is_none_or(|id| id.is_empty()) {
            return Err(AppError::new("INVALID_ARGUMENT", "请选择一个会话"));
        }
        let projection = self.browse_projection(&context, &all, &lease)?;
        if projection.is_some() {
            context.query_strategy = "protected_batch_projection".into();
        }
        let projected = projection
            .as_ref()
            .map(|p| (p, q.conversation_id.as_deref().unwrap_or_default()));
        let secret = self.history_index_secret(account)?;
        let boundary = q
            .cursor
            .as_ref()
            .map(|c| {
                unseal(
                    c,
                    &secret,
                    &context.generation,
                    direction_name(request.direction),
                )
            })
            .transpose()?;
        if matches!(
            request.direction,
            BrowseDirection::Older | BrowseDirection::Newer
        ) && boundary.is_none()
        {
            return Err(AppError::new("INVALID_ARGUMENT", "翻页缺少游标"));
        }
        let search = q.search.as_ref().map(|s| s.to_lowercase());
        let mut anchor = None;
        if request.direction == BrowseDirection::Around {
            if let Some(id) = &request.anchor_id {
                self.browse_scan(
                    &context,
                    &all,
                    &lease,
                    projected,
                    &|m| matches(m, q, &search) && &m.id == id,
                    &mut |m| {
                        anchor = Some(key(&m));
                        Ok(())
                    },
                )?;
                if anchor.is_none() {
                    return Err(AppError::new(
                        "MESSAGE_NOT_FOUND",
                        "此范围内没有找到消息锚点",
                    ));
                }
            } else if let Some(timestamp) = request.anchor_timestamp {
                anchor = Some((timestamp, i64::MIN, String::new()));
            } else {
                return Err(AppError::new("INVALID_ARGUMENT", "定位需要日期或消息ID"));
            }
        }
        let mut before = BTreeMap::new();
        let mut after = BTreeMap::new();
        let mut budget = QueryBudget::default();
        let half = q.limit as usize;
        self.browse_scan(
            &context,
            &all,
            &lease,
            projected,
            &|m| matches(m, q, &search),
            &mut |message| {
                let order = key(&message);
                let pick_after = match request.direction {
                    BrowseDirection::Latest => false,
                    BrowseDirection::Older => {
                        if boundary.as_ref().is_some_and(|b| order >= *b) {
                            return Ok(());
                        }
                        false
                    }
                    BrowseDirection::Newer => {
                        if boundary.as_ref().is_some_and(|b| order <= *b) {
                            return Ok(());
                        }
                        true
                    }
                    BrowseDirection::Around => anchor.as_ref().is_some_and(|a| order >= *a),
                };
                let page = if pick_after { &mut after } else { &mut before };
                let new = retained_bytes(&message)?;
                let old = page
                    .insert(order, message)
                    .map(|m| retained_bytes(&m))
                    .transpose()?
                    .unwrap_or(0);
                let removed = if page.len() > half + 1 {
                    if pick_after {
                        page.pop_last()
                    } else {
                        page.pop_first()
                    }
                } else {
                    None
                };
                budget.replace(
                    old + removed
                        .map(|(_, m)| retained_bytes(&m))
                        .transpose()?
                        .unwrap_or(0),
                    new,
                )?;
                Ok(())
            },
        )?;
        let (keep_before, keep_after) = if request.direction == BrowseDirection::Around {
            let left = before.len().min(half.div_ceil(2));
            let right = after.len().min(half - left);
            (before.len().min(half - right), right)
        } else {
            (half, half)
        };
        let more_before = before.len() > keep_before;
        let more_after = after.len() > keep_after;
        while before.len() > keep_before {
            before.pop_first();
        }
        while after.len() > keep_after {
            after.pop_last();
        }
        let mut rows = before;
        rows.extend(after);
        let cursor = |purpose: &str, order: MessageKey| {
            seal(
                BrowseCursor {
                    version: 1,
                    fingerprint: context.generation.clone(),
                    purpose: purpose.into(),
                    key: order,
                },
                &secret,
            )
        };
        let older_cursor =
            if !rows.is_empty() && (more_before || request.direction == BrowseDirection::Newer) {
                Some(cursor(
                    "older",
                    rows.first_key_value()
                        .ok_or_else(AppError::internal)?
                        .0
                        .clone(),
                )?)
            } else {
                None
            };
        let newer_cursor =
            if !rows.is_empty() && (more_after || request.direction == BrowseDirection::Older) {
                Some(cursor(
                    "newer",
                    rows.last_key_value()
                        .ok_or_else(AppError::internal)?
                        .0
                        .clone(),
                )?)
            } else {
                None
            };
        let (contacts, _) =
            self.browse_metadata(&context, &lease, &mut budget, projection.as_ref())?;
        let data = rows
            .into_values()
            .map(|m| {
                let old = retained_bytes(&m)?;
                let sender_name = m
                    .sender_id
                    .as_ref()
                    .and_then(|id| contacts.get(id))
                    .and_then(|c| {
                        c.remark
                            .clone()
                            .filter(|s| !s.is_empty())
                            .or_else(|| c.nickname.clone().filter(|s| !s.is_empty()))
                    });
                let view = MessageView {
                    schema_version: 1,
                    id: m.id,
                    conversation_id: m.conversation_id,
                    sender_id: m.sender_id,
                    sender_name,
                    direction: "unknown".into(),
                    identity_status: "unverified".into(),
                    server_id: m.server_id,
                    local_id: m.local_id.to_string(),
                    sequence: m.sequence.to_string(),
                    timestamp: m.timestamp,
                    message_type: m.message_type.to_string(),
                    text: m.text,
                    content_status: m.content_status,
                };
                budget.replace(old, retained_bytes(&view)?)?;
                Ok(view)
            })
            .collect::<AppResult<Vec<_>>>()?;
        self.check_browse_context(&context, &all, &lease)?;
        if projection.is_some() {
            context.query_metrics = Some(lease.metrics());
        }
        Ok(TimelinePage {
            data,
            older_cursor,
            newer_cursor,
            context,
        })
    }
}
