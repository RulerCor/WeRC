use super::browse::QueryLease;
use super::*;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use hmac::{Hmac, Mac};
use werc_domain::{browse::ReadContext, projection::*};
use zeroize::Zeroizing;

type ProjectedBatchVisitor<'a> = dyn FnMut(Vec<HistoryMessage>, &[u16]) -> AppResult<()> + 'a;

// 定位只保留源/批次/行号；正文仍在原认证历史批次中，不复制到索引。
#[derive(Serialize, Deserialize)]
pub(super) struct Location(u16, u32, u16);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Header {
    version: u32,
    pub contacts: BTreeMap<String, HistoryContact>,
    pub sessions: BTreeMap<String, HistorySession>,
    rows: BTreeMap<String, String>,
}
pub(super) struct Projection {
    protected: ProtectedProjection,
    key: Zeroizing<Vec<u8>>,
    pub header: Header,
}
fn corrupted() -> AppError {
    AppError::new(
        "PROJECTION_CORRUPTED",
        "投影认证失败；已放弃缓存，请重新准备",
    )
}
fn aad(data: &ProtectedProjection, token: &str) -> AppResult<Vec<u8>> {
    serde_json::to_vec(&(
        1,
        "WeRC/history-projection",
        &data.account_id,
        &data.id,
        &data.generation,
        token,
    ))
    .map_err(|_| AppError::internal())
}
fn encrypt(
    data: &ProtectedProjection,
    key: &[u8],
    token: &str,
    value: &impl Serialize,
    limit: usize,
) -> AppResult<(Vec<u8>, Vec<u8>)> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| AppError::internal())?);
    if bytes.len() > limit {
        return Err(AppError::new(
            "QUOTA_EXCEEDED",
            "投影分区超过保护预算，请缩小快照范围",
        ));
    }
    let mut nonce = vec![0; 12];
    getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
    let ciphertext = Aes256Gcm::new_from_slice(key)
        .map_err(|_| AppError::internal())?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &bytes,
                aad: &aad(data, token)?,
            },
        )
        .map_err(|_| AppError::internal())?;
    Ok((nonce, ciphertext))
}
fn decrypt<T: serde::de::DeserializeOwned>(
    data: &ProtectedProjection,
    key: &[u8],
    token: &str,
    nonce: &[u8],
    ciphertext: &[u8],
    limit: usize,
) -> AppResult<T> {
    if nonce.len() != 12 || ciphertext.len() > limit + 16 {
        return Err(corrupted());
    }
    let bytes = Zeroizing::new(
        Aes256Gcm::new_from_slice(key)
            .map_err(|_| corrupted())?
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad(data, token)?,
                },
            )
            .map_err(|_| corrupted())?,
    );
    serde_json::from_slice(&bytes).map_err(|_| corrupted())
}
fn token(key: &[u8], conversation: &str) -> AppResult<String> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).map_err(|_| AppError::internal())?;
    mac.update(b"WeRC/history-projection-conversation/v1\0");
    mac.update(conversation.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}
impl Application {
    pub fn clear_browse_cache_local(&self, account: &str) -> AppResult<usize> {
        self.ensure_account(account)?;
        let _guard = self
            .projection_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "正在准备查询缓存，请取消并等待读取结束后重试"))?;
        self.repository.clear_projections(account)
    }
    fn projection_generation(
        &self,
        context: &ReadContext,
        all: &[HistorySource],
        lease: &QueryLease<'_>,
    ) -> AppResult<Option<String>> {
        let mut hash = Sha256::new();
        hash.update(
            serde_json::to_vec(&(
                1,
                &context.account_id,
                &context.connection_revision,
                &context.sources,
                all,
            ))
            .map_err(|_| AppError::internal())?,
        );
        for source in all {
            lease.check()?;
            let data = self
                .repository
                .history_payload(&context.account_id, &source.id)?;
            // v1 仍用原预算扫描，不能将未分批旧格式称随机定位。
            if data.version != 2 {
                return Ok(None);
            }
            for value in [&data.wrapped_key, &data.nonce, &data.ciphertext] {
                hash.update((value.len() as u64).to_le_bytes());
                hash.update(value);
            }
            hash.update(serde_json::to_vec(&data.source).map_err(|_| AppError::internal())?);
        }
        Ok(Some(hex::encode(hash.finalize())))
    }
    pub(super) fn browse_projection(
        &self,
        context: &ReadContext,
        all: &[HistorySource],
        lease: &QueryLease<'_>,
    ) -> AppResult<Option<Projection>> {
        let Some(generation) = self.projection_generation(context, all, lease)? else {
            return Ok(None);
        };
        let data = if let Some(data) = self
            .repository
            .projection(&context.account_id, &generation)?
        {
            data
        } else {
            // A source change cancels the previous UI query while its worker may
            // still be releasing the builder. Keep this admitted read pending
            // within its existing deadline instead of leaving the new view empty.
            let _guard = loop {
                lease.check()?;
                match self.projection_guard.try_lock() {
                    Ok(guard) => break guard,
                    Err(std::sync::TryLockError::WouldBlock) => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(std::sync::TryLockError::Poisoned(_)) => {
                        return Err(AppError::internal());
                    }
                }
            };
            lease.check()?;
            if let Some(data) = self
                .repository
                .projection(&context.account_id, &generation)?
            {
                data
            } else {
                self.build_projection(context, all, lease, &generation)?;
                self.repository
                    .projection(&context.account_id, &generation)?
                    .ok_or_else(corrupted)?
            }
        };
        let decoded = (|| {
            if data.wrapped_key.is_empty()
                || data.wrapped_key.len() > 16 * 1024
                || data.nonce.len() != 12
                || data.ciphertext.len() < 16
                || data.ciphertext.len() > PROJECTION_HEADER_BYTES + 16
            {
                return Err(corrupted());
            }
            let key = Zeroizing::new(self.provider.unprotect_key(
                &format!("WeRC/history-projection/v1/{}/{}", data.account_id, data.id),
                &data.wrapped_key,
            )?);
            let header: Header = decrypt(
                &data,
                &key,
                "header",
                &data.nonce,
                &data.ciphertext,
                PROJECTION_HEADER_BYTES,
            )?;
            if header.version != 1 {
                return Err(corrupted());
            }
            Ok((key, header))
        })();
        match decoded {
            Ok((key, header)) => Ok(Some(Projection {
                protected: data,
                key,
                header,
            })),
            Err(error) => {
                self.repository
                    .remove_projection(&context.account_id, &data.id)?;
                Err(error)
            }
        }
    }
    fn build_projection(
        &self,
        context: &ReadContext,
        all: &[HistorySource],
        lease: &QueryLease<'_>,
        generation: &str,
    ) -> AppResult<()> {
        let account = &context.account_id;
        lease.projection_built.set(true);
        let secret = self.history_index_secret(account)?;
        let mut header = Header {
            version: 1,
            contacts: BTreeMap::new(),
            sessions: BTreeMap::new(),
            rows: BTreeMap::new(),
        };
        let mut locations: BTreeMap<String, Vec<Location>> = BTreeMap::new();
        let mut metadata_budget = QueryBudget::default();
        let mut locator_bytes = 0usize;
        let aggregate = context.sources.len() == all.len();
        for (source_index, source) in context.sources.iter().enumerate() {
            let source_index = u16::try_from(source_index).map_err(|_| AppError::internal())?;
            let mut sequence = 0u64;
            self.visit_history_source_controlled(source, &|| lease.check(), &mut |bundle| {
                lease.check()?;
                lease.batches.set(lease.batches.get() + 1);
                for contact in bundle.contacts {
                    let old = header
                        .contacts
                        .get(&contact.username)
                        .map(retained_bytes)
                        .transpose()?
                        .unwrap_or(0);
                    metadata_budget.replace(old, retained_bytes(&contact)?)?;
                    header.contacts.insert(contact.username.clone(), contact);
                }
                for session in bundle.sessions {
                    if header
                        .sessions
                        .get(&session.username)
                        .is_some_and(|old| old.timestamp > session.timestamp)
                    {
                        continue;
                    }
                    let old = header
                        .sessions
                        .get(&session.username)
                        .map(retained_bytes)
                        .transpose()?
                        .unwrap_or(0);
                    metadata_budget.replace(old, retained_bytes(&session)?)?;
                    header.sessions.insert(session.username.clone(), session);
                }
                let ids: Vec<_> = bundle.messages.iter().map(|m| m.id.clone()).collect();
                let states = if aggregate {
                    self.repository.history_message_states(account, &ids)?
                } else {
                    BTreeMap::new()
                };
                let mut groups: BTreeMap<String, Vec<u16>> = BTreeMap::new();
                for (row, message) in bundle.messages.into_iter().enumerate() {
                    if aggregate {
                        let digest = crate::history_stream::message_digest(&secret, &message)?;
                        if states
                            .get(&message.id)
                            .is_none_or(|s| s.source_id != source.id || s.digest != digest)
                        {
                            continue;
                        }
                    }
                    let conversation_token = token(&secret, &message.conversation_id)?;
                    groups
                        .entry(conversation_token)
                        .or_default()
                        .push(u16::try_from(row).map_err(|_| corrupted())?);
                    let old = header
                        .sessions
                        .get(&message.conversation_id)
                        .map(retained_bytes)
                        .transpose()?
                        .unwrap_or(0);
                    let session = header
                        .sessions
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
                    metadata_budget.replace(old, retained_bytes(session)?)?;
                }
                for (token, rows) in groups {
                    // 独立于32MiB结果预算：单个构建的定位工作集最多96MiB估算值。
                    locator_bytes = locator_bytes
                        .checked_add(if locations.contains_key(&token) {
                            0
                        } else {
                            256
                        })
                        .and_then(|n| n.checked_add(rows.len() * 32))
                        .ok_or_else(AppError::internal)?;
                    if locator_bytes > 96 * 1024 * 1024 {
                        return Err(AppError::new(
                            "QUOTA_EXCEEDED",
                            "投影定位工作集超过96MiB，请缩小快照范围",
                        ));
                    }
                    let batch = u32::try_from(sequence).map_err(|_| corrupted())?;
                    locations.entry(token).or_default().extend(
                        rows.into_iter()
                            .map(|row| Location(source_index, batch, row)),
                    );
                }
                sequence += 1;
                Ok(())
            })?;
        }
        self.check_browse_context(context, all, lease)?;
        if self.projection_generation(context, all, lease)?.as_deref() != Some(generation) {
            return Err(AppError::new("CURSOR_STALE", "准备期间来源保护清单已变化"));
        }
        let key = Zeroizing::new({
            let mut k = vec![0; 32];
            getrandom::fill(&mut k).map_err(|_| AppError::internal())?;
            k
        });
        let id = uuid::Uuid::new_v4().to_string();
        let mut data = ProtectedProjection {
            id: id.clone(),
            account_id: account.clone(),
            generation: generation.into(),
            wrapped_key: self
                .provider
                .protect_key(&format!("WeRC/history-projection/v1/{account}/{id}"), &key)?,
            nonce: vec![],
            ciphertext: vec![],
        };
        self.repository.begin_projection(&data)?;
        let result = (|| {
            let mut pending = Vec::new();
            let mut pending_bytes = 0usize;
            for (token, rows) in locations {
                lease.check()?;
                let (nonce, ciphertext) =
                    encrypt(&data, &key, &token, &rows, PROJECTION_ROW_BYTES)?;
                header
                    .rows
                    .insert(token.clone(), hex::encode(Sha256::digest(&ciphertext)));
                if !pending.is_empty()
                    && (pending.len() == 64
                        || pending_bytes + ciphertext.len() > PROJECTION_ROW_BYTES + 16)
                {
                    self.repository.append_projection(account, &id, &pending)?;
                    pending.clear();
                    pending_bytes = 0;
                    lease.check()?;
                }
                pending_bytes += ciphertext.len();
                pending.push(ProtectedProjectionRow {
                    token,
                    nonce,
                    ciphertext,
                });
            }
            lease.check()?;
            if !pending.is_empty() {
                self.repository.append_projection(account, &id, &pending)?;
            }
            lease.check()?;
            (data.nonce, data.ciphertext) =
                encrypt(&data, &key, "header", &header, PROJECTION_HEADER_BYTES)?;
            self.check_browse_context(context, all, lease)?;
            self.repository.finish_projection(&data)
        })();
        if result.is_err() {
            self.repository.remove_projection(account, &id)?;
        }
        result
    }
    pub(super) fn scan_projection(
        &self,
        projection: &Projection,
        scope: (&ReadContext, &str),
        lease: &QueryLease<'_>,
        filter: &dyn Fn(&HistoryMessage) -> bool,
        visitor: &mut dyn FnMut(HistoryMessage) -> AppResult<()>,
    ) -> AppResult<()> {
        let (context, conversation) = scope;
        let secret = self.history_index_secret(&context.account_id)?;
        let token = token(&secret, conversation)?;
        let Some(expected) = projection.header.rows.get(&token) else {
            return Ok(());
        };
        let result = (|| {
            let row = self
                .repository
                .projection_row(&context.account_id, &projection.protected.id, &token)?
                .ok_or_else(corrupted)?;
            if hex::encode(Sha256::digest(&row.ciphertext)) != *expected {
                return Err(corrupted());
            }
            let rows: Vec<Location> = decrypt(
                &projection.protected,
                &projection.key,
                &token,
                &row.nonce,
                &row.ciphertext,
                PROJECTION_ROW_BYTES,
            )?;
            let mut grouped: BTreeMap<u16, BTreeMap<u64, Vec<u16>>> = BTreeMap::new();
            for Location(source, batch, row) in rows {
                grouped
                    .entry(source)
                    .or_default()
                    .entry(u64::from(batch))
                    .or_default()
                    .push(row);
            }
            for (source, locations) in grouped {
                lease.check()?;
                let source = context.sources.get(source as usize).ok_or_else(corrupted)?;
                let locations: Vec<_> = locations.into_iter().collect();
                self.visit_projected_batches(
                    source,
                    &locations,
                    lease,
                    &mut |messages, indices| {
                        for (row, message) in messages.into_iter().enumerate() {
                            if indices.binary_search(&(row as u16)).is_ok() {
                                if message.conversation_id != conversation {
                                    return Err(corrupted());
                                }
                                if filter(&message) {
                                    visitor(message)?;
                                }
                            }
                        }
                        Ok(())
                    },
                )?;
            }
            Ok(())
        })();
        if result
            .as_ref()
            .is_err_and(|error| error.code == "PROJECTION_CORRUPTED")
        {
            self.repository
                .remove_projection(&context.account_id, &projection.protected.id)?;
        }
        result
    }
    fn visit_projected_batches(
        &self,
        source: &HistorySource,
        locations: &[(u64, Vec<u16>)],
        lease: &QueryLease<'_>,
        visitor: &mut ProjectedBatchVisitor<'_>,
    ) -> AppResult<()> {
        let data = self
            .repository
            .history_payload(&source.account_id, &source.id)?;
        if data.version != 2 || data.nonce.len() != 12 {
            return Err(corrupted());
        }
        let key = Zeroizing::new(self.provider.unprotect_key(
            &crate::history_stream::context(source, 2),
            &data.wrapped_key,
        )?);
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| corrupted())?;
        let aad = serde_json::to_vec(&data.source).map_err(|_| AppError::internal())?;
        let manifest = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&data.nonce),
                    Payload {
                        msg: &data.ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| corrupted())?,
        );
        let (marker, count, _digest): (String, u64, String) =
            serde_json::from_slice(&manifest).map_err(|_| corrupted())?;
        if marker != "WeRC/history-manifest/v2" {
            return Err(corrupted());
        }
        // 定位来自经过完整来源/摘要索引验证后密封的投影。热页只认证实际访问的原批次，
        // 不把对其他未访问批次的损坏检测称为本次验证结果。
        for (sequence, indices) in locations {
            lease.check()?;
            if *sequence >= count || indices.is_empty() {
                return Err(corrupted());
            }
            let batch = self
                .repository
                .history_batch(&source.account_id, &source.id, *sequence)?
                .ok_or_else(corrupted)?;
            if batch.nonce.len() != 12 || batch.ciphertext.len() > HISTORY_STORED_BATCH_BYTES + 16 {
                return Err(corrupted());
            }
            let bytes = Zeroizing::new(
                cipher
                    .decrypt(
                        Nonce::from_slice(&batch.nonce),
                        Payload {
                            msg: &batch.ciphertext,
                            aad: &crate::history_stream::batch_aad(source, *sequence)?,
                        },
                    )
                    .map_err(|_| AppError::new("SOURCE_CORRUPTED", "目标历史批次认证失败"))?,
            );
            let bundle: HistoryBundle = serde_json::from_slice(&bytes).map_err(|_| corrupted())?;
            if bundle.schema != source.schema
                || bundle.source_sha256 != source.source_sha256
                || bundle.encryption != source.encryption
                || indices
                    .last()
                    .is_none_or(|i| *i as usize >= bundle.messages.len())
                || indices.windows(2).any(|p| p[0] >= p[1])
            {
                return Err(corrupted());
            }
            lease.batches.set(lease.batches.get() + 1);
            visitor(bundle.messages, indices)?;
        }
        Ok(())
    }
}
