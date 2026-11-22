use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use werc_domain::{AppError, AppResult, history::*};
use zeroize::Zeroizing;

fn protocol() -> AppError {
    AppError::new("READER_PROTOCOL_INVALID", "读取批次、顺序或完成清单不匹配")
}
pub(crate) fn batch_aad(source: &HistorySource, sequence: u64) -> AppResult<Vec<u8>> {
    serde_json::to_vec(&(
        2,
        &source.account_id,
        &source.id,
        &source.source_sha256,
        sequence,
    ))
    .map_err(|_| AppError::internal())
}
pub(crate) fn context(source: &HistorySource, version: u32) -> String {
    format!(
        "WeRC/history/v{version}/{}/{}",
        source.account_id, source.id
    )
}
pub(super) fn message_digest(key: &[u8], message: &HistoryMessage) -> AppResult<String> {
    use hmac::Mac;
    let bytes = Zeroizing::new(serde_json::to_vec(message).map_err(|_| AppError::internal())?);
    let mut mac =
        <hmac::Hmac<Sha256> as Mac>::new_from_slice(key).map_err(|_| AppError::internal())?;
    mac.update(bytes.as_slice());
    Ok(hex::encode(mac.finalize().into_bytes()))
}
fn normalize(message: &mut HistoryMessage) {
    let identity = if message.server_id != "0" {
        format!("{}\nserver:{}", message.conversation_id, message.server_id)
    } else {
        format!(
            "{}\nlocal:{}:{}",
            message.conversation_id, message.local_id, message.timestamp
        )
    };
    message.id = hex::encode(Sha256::digest(identity.as_bytes()));
}

impl Application {
    pub(super) fn history_index_secret(&self, account: &str) -> AppResult<Zeroizing<Vec<u8>>> {
        let context = format!("WeRC/history-index/v1/{account}");
        let wrapped = if let Some(wrapped) = self.repository.history_index_key(account, None)? {
            wrapped
        } else {
            for source in self.history_sources_local(account)? {
                if self
                    .repository
                    .history_payload(account, &source.id)?
                    .version
                    == 2
                {
                    return Err(AppError::new("SOURCE_CORRUPTED", "历史索引保护密钥缺失"));
                }
            }
            let mut key = Zeroizing::new([0u8; 32]);
            getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
            let wrapped = self.provider.protect_key(&context, key.as_ref())?;
            self.repository
                .history_index_key(account, Some(&wrapped))?
                .ok_or_else(AppError::internal)?
        };
        let key = Zeroizing::new(self.provider.unprotect_key(&context, &wrapped)?);
        if key.len() != 32 {
            return Err(AppError::new(
                "SOURCE_CORRUPTED",
                "历史索引保护密钥格式无效",
            ));
        }
        Ok(key)
    }
    fn history_request_key(
        &self,
        account: &str,
        request: &mut HistoryReadRequest,
    ) -> AppResult<()> {
        if request.allow_plaintext || request.key_hex.as_ref().is_some_and(|s| !s.is_empty()) {
            return Ok(());
        }
        let binding = self
            .repository
            .bindings()?
            .into_iter()
            .find(|b| b.account_id == account)
            .ok_or_else(|| AppError::new("KEY_REQUIRED", "请先绑定并获取此来源的密钥"))?;
        let (protected, _) = self
            .repository
            .protected_key(account)?
            .ok_or_else(|| AppError::new("KEY_REQUIRED", "没有可用的受保护密钥"))?;
        let key = Zeroizing::new(self.provider.unprotect_key(
            &format!("WeRC/key/v1/{account}/{}", binding.installation.sha256),
            &protected,
        )?);
        let salt = if key.len() == 32 {
            [0; 16]
        } else {
            self.provider.database_salt(&request.path)?
        };
        let selected = crate::keyring::key_for(&key, &salt)?;
        request.key_hex = Some(hex::encode(selected.as_slice()));
        Ok(())
    }
    pub fn import_history_local(
        &self,
        account: &str,
        request: HistoryReadRequest,
        confirmed: bool,
    ) -> AppResult<HistorySource> {
        self.import_history_controlled(account, request, confirmed, false)
    }
    pub(super) fn import_history_controlled(
        &self,
        account: &str,
        mut request: HistoryReadRequest,
        confirmed: bool,
        setup: bool,
    ) -> AppResult<HistorySource> {
        let _guard = if setup {
            // Automatic setup yields to interactive source operations.
            self.history_guard
                .try_lock()
                .map_err(|_| AppError::new("BUSY", "已有来源正在导入，请等待其完成"))?
        } else {
            // A user-requested import joins the same fair queue as other foreground reads.
            self.history_guard.exclusive(|| Ok(()))?
        };
        self.ensure_account(account)?;
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "请明确确认读取此离线来源",
            ));
        }
        let revision = self.repository.key_revision(account)?;
        self.ensure_current_connection(account)?;
        self.history_request_key(account, &mut request)?;
        let index_key = self.history_index_secret(account)?;
        let mut legacy = std::collections::BTreeMap::new();
        for source in self.history_sources_local(account)? {
            if self
                .repository
                .history_payload(account, &source.id)?
                .version
                == 1
            {
                self.visit_history_source(&source, &mut |bundle| {
                    for message in bundle.messages {
                        legacy.insert(message.id.clone(), message_digest(&index_key, &message)?);
                    }
                    Ok(())
                })?;
            }
        }
        let mut source: Option<HistorySource> = None;
        let mut key = Zeroizing::new([0u8; 32]);
        let mut wrapped_key = Vec::new();
        let mut sequence = 0u64;
        let mut counts = (0usize, 0usize, 0usize);
        let mut ended = false;
        let mut duplicate = None;
        let result = self.provider.stream_database(&request, &mut |event| {
            if setup && !self.setup_active.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(AppError::new("SETUP_PAUSED", "引导已暂停，未提交当前来源"));
            }
            if ended || self.repository.key_revision(account)? != revision {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "导入期间密钥或来源授权已改变",
                ));
            }
            match event {
                HistoryReadEvent::Begin {
                    schema,
                    source_sha256,
                    encryption,
                    ..
                } => {
                    if source.is_some()
                        || source_sha256.len() != 64
                        || !source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                        || schema.len() > 128
                        || encryption.len() > 128
                    {
                        return Err(protocol());
                    }
                    if let Some(old) = self
                        .history_sources_local(account)?
                        .into_iter()
                        .find(|s| s.source_sha256 == source_sha256)
                    {
                        duplicate = Some(old);
                        return Err(AppError::new(
                            "HISTORY_ALREADY_IMPORTED",
                            "相同快照已经导入",
                        ));
                    }
                    let next = HistorySource {
                        id: Uuid::new_v4().to_string(),
                        account_id: account.into(),
                        source_sha256,
                        schema,
                        encryption,
                        message_count: 0,
                        contact_count: 0,
                        session_count: 0,
                        imported_at: Utc::now().to_rfc3339(),
                        real_wechat_verified: false,
                    };
                    getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
                    wrapped_key = self
                        .provider
                        .protect_key(&context(&next, 2), key.as_ref())?;
                    self.repository.begin_history(&ProtectedHistory {
                        version: 2,
                        changes: Vec::new(),
                        source: next.clone(),
                        wrapped_key: wrapped_key.clone(),
                        nonce: vec![0; 12],
                        ciphertext: Vec::new(),
                    })?;
                    source = Some(next);
                }
                HistoryReadEvent::Batch {
                    sequence: received,
                    digest,
                    mut data,
                } => {
                    let current = source.as_ref().ok_or_else(protocol)?;
                    let raw = Zeroizing::new(serde_json::to_vec(&data).map_err(|_| protocol())?);
                    if sequence >= 1_000_000
                        || received != sequence
                        || data.source_sha256 != current.source_sha256
                        || data.schema != current.schema
                        || data.encryption != current.encryption
                        || data.warnings.len() > 128
                        || raw.len() > HISTORY_BATCH_BYTES
                        || hex::encode(Sha256::digest(raw.as_slice())) != digest
                        || data.messages.len() + data.contacts.len() + data.sessions.len()
                            > HISTORY_BATCH_ROWS
                    {
                        return Err(protocol());
                    }
                    for message in &mut data.messages {
                        normalize(message);
                    }
                    let ids: Vec<String> = data
                        .messages
                        .iter()
                        .map(|message| message.id.clone())
                        .collect();
                    let states = self.repository.history_message_states(account, &ids)?;
                    let mut records = Vec::new();
                    let mut changes = Vec::new();
                    for message in &data.messages {
                        let digest = message_digest(&index_key, message)?;
                        let previous = states
                            .get(&message.id)
                            .map(|state| state.digest.clone())
                            .or_else(|| legacy.get(&message.id).cloned());
                        let change = match previous {
                            None => Some("added"),
                            Some(old) if old != digest => Some("updated"),
                            _ => None,
                        };
                        if let Some(change) = change {
                            changes.push(HistoryChange {
                                message_id: message.id.clone(),
                                change,
                            });
                        }
                        records.push(HistoryRecordDigest {
                            message_id: message.id.clone(),
                            digest,
                        });
                    }
                    let bytes = Zeroizing::new(serde_json::to_vec(&data).map_err(|_| protocol())?);
                    if bytes.len() > HISTORY_STORED_BATCH_BYTES {
                        return Err(protocol());
                    }
                    let mut nonce = [0; 12];
                    getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
                    let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
                        .map_err(|_| AppError::internal())?
                        .encrypt(
                            Nonce::from_slice(&nonce),
                            Payload {
                                msg: bytes.as_slice(),
                                aad: &batch_aad(current, sequence)?,
                            },
                        )
                        .map_err(|_| AppError::internal())?;
                    self.repository.append_history_batch(
                        account,
                        &current.id,
                        &ProtectedHistoryBatch {
                            sequence,
                            nonce: nonce.to_vec(),
                            ciphertext,
                            records,
                            changes,
                        },
                    )?;
                    counts.0 = counts
                        .0
                        .checked_add(data.messages.len())
                        .ok_or_else(protocol)?;
                    counts.1 = counts
                        .1
                        .checked_add(data.contacts.len())
                        .ok_or_else(protocol)?;
                    counts.2 = counts
                        .2
                        .checked_add(data.sessions.len())
                        .ok_or_else(protocol)?;
                    sequence += 1;
                }
                HistoryReadEvent::End {
                    batches,
                    message_count,
                    contact_count,
                    session_count,
                } => {
                    if source.is_none()
                        || batches != sequence
                        || counts != (message_count, contact_count, session_count)
                    {
                        return Err(protocol());
                    }
                    ended = true;
                }
                HistoryReadEvent::Error { code } => {
                    return Err(AppError::new(&code, "来源读取未完成，未提交导入结果"));
                }
            }
            Ok(())
        });
        if let Some(old) = duplicate {
            return Ok(old);
        }
        let outcome = (|| {
            result?;
            if !ended {
                return Err(protocol());
            }
            if setup && !self.setup_active.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(AppError::new("SETUP_PAUSED", "引导已暂停，未提交当前来源"));
            }
            if self.repository.key_revision(account)? != revision {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "导入期间密钥授权已改变",
                ));
            }
            let current = source.as_mut().ok_or_else(protocol)?;
            current.message_count = counts.0;
            current.contact_count = counts.1;
            current.session_count = counts.2;
            let mut nonce = [0; 12];
            getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
            let aad = serde_json::to_vec(current).map_err(|_| AppError::internal())?;
            let manifest = Zeroizing::new(
                serde_json::to_vec(&(
                    "WeRC/history-manifest/v2",
                    sequence,
                    self.repository.history_index_digest(account, &current.id)?,
                ))
                .map_err(|_| AppError::internal())?,
            );
            let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
                .map_err(|_| AppError::internal())?
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: manifest.as_slice(),
                        aad: &aad,
                    },
                )
                .map_err(|_| AppError::internal())?;
            self.repository.finish_history(
                &ProtectedHistory {
                    version: 2,
                    changes: Vec::new(),
                    source: current.clone(),
                    wrapped_key,
                    nonce: nonce.to_vec(),
                    ciphertext,
                },
                revision,
            )?;
            Ok(current.clone())
        })();
        if outcome.is_err()
            && let Some(current) = &source
        {
            self.repository.abort_history(account, &current.id)?;
        }
        outcome
    }
    pub(super) fn visit_history_source(
        &self,
        source: &HistorySource,
        visitor: &mut dyn FnMut(HistoryBundle) -> AppResult<()>,
    ) -> AppResult<()> {
        self.visit_history_source_controlled(source, &|| Ok(()), visitor)
    }
    pub(super) fn visit_history_source_controlled(
        &self,
        source: &HistorySource,
        check: &dyn Fn() -> AppResult<()>,
        visitor: &mut dyn FnMut(HistoryBundle) -> AppResult<()>,
    ) -> AppResult<()> {
        check()?;
        let protected = self
            .repository
            .history_payload(&source.account_id, &source.id)?;
        if protected.nonce.len() != 12
            || protected.source.account_id != source.account_id
            || protected.source.source_sha256 != source.source_sha256
        {
            return Err(AppError::new("SOURCE_CORRUPTED", "来源保护清单不匹配"));
        }
        let key = Zeroizing::new(
            self.provider
                .unprotect_key(&context(source, protected.version), &protected.wrapped_key)?,
        );
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| AppError::internal())?;
        let aad = serde_json::to_vec(&protected.source).map_err(|_| AppError::internal())?;
        let bytes = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&protected.nonce),
                    Payload {
                        msg: &protected.ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| AppError::new("SOURCE_CORRUPTED", "来源保护内容认证失败"))?,
        );
        if protected.version == 1 {
            let bundle: HistoryBundle = serde_json::from_slice(&bytes).map_err(|_| protocol())?;
            if bundle.source_sha256 != source.source_sha256 {
                return Err(protocol());
            }
            return visitor(bundle);
        }
        if protected.version != 2 {
            return Err(protocol());
        }
        let (marker, batch_count, index_digest): (String, u64, String) =
            serde_json::from_slice(&bytes).map_err(|_| protocol())?;
        if marker != "WeRC/history-manifest/v2"
            || batch_count > 1_000_000
            || index_digest
                != self.repository.history_index_digest_controlled(
                    &source.account_id,
                    &source.id,
                    check,
                )?
        {
            return Err(AppError::new(
                "SOURCE_CORRUPTED",
                "历史清单或消息索引认证失败",
            ));
        }
        let mut counts = (0usize, 0usize, 0usize);
        let mut sequence = 0;
        while sequence < batch_count {
            check()?;
            let batch = self
                .repository
                .history_batch(&source.account_id, &source.id, sequence)?
                .ok_or_else(|| AppError::new("SOURCE_CORRUPTED", "历史批次缺失"))?;
            if batch.nonce.len() != 12 || batch.ciphertext.len() > HISTORY_STORED_BATCH_BYTES + 16 {
                return Err(protocol());
            }
            let bytes = Zeroizing::new(
                cipher
                    .decrypt(
                        Nonce::from_slice(&batch.nonce),
                        Payload {
                            msg: &batch.ciphertext,
                            aad: &batch_aad(source, sequence)?,
                        },
                    )
                    .map_err(|_| AppError::new("SOURCE_CORRUPTED", "历史批次完整性校验失败"))?,
            );
            let bundle: HistoryBundle = serde_json::from_slice(&bytes).map_err(|_| protocol())?;
            if bundle.source_sha256 != source.source_sha256
                || bundle.schema != source.schema
                || bundle.encryption != source.encryption
            {
                return Err(protocol());
            }
            counts.0 += bundle.messages.len();
            counts.1 += bundle.contacts.len();
            counts.2 += bundle.sessions.len();
            visitor(bundle)?;
            sequence += 1;
        }
        if self
            .repository
            .history_batch(&source.account_id, &source.id, batch_count)?
            .is_some()
            || counts
                != (
                    source.message_count,
                    source.contact_count,
                    source.session_count,
                )
        {
            return Err(AppError::new(
                "SOURCE_CORRUPTED",
                "历史批次缺失或计数不匹配",
            ));
        }
        Ok(())
    }
    pub(super) fn visit_history(
        &self,
        account: &str,
        visitor: &mut dyn FnMut(&HistorySource, HistoryBundle) -> AppResult<()>,
    ) -> AppResult<Vec<HistorySource>> {
        let mut sources = self.history_sources_local(account)?;
        sources.sort_by(|a, b| a.imported_at.cmp(&b.imported_at).then(a.id.cmp(&b.id)));
        for source in &sources {
            self.visit_history_source(source, &mut |bundle| visitor(source, bundle))?;
        }
        Ok(sources)
    }
}
