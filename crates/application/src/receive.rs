use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use werc_domain::{AppError, AppResult, Principal, receive::*};
use zeroize::Zeroizing;

fn corrupt() -> AppError {
    AppError::new("RECEIVE_CORRUPTED", "实时接收记录认证失败")
}
fn scope(account: &str, id: &str) -> String {
    format!("WeRC/received-text/v1/{account}/{id}")
}

impl Application {
    pub fn receive_health_local(&self, account: &str) -> AppResult<Option<ReceiveHealth>> {
        if self.repository.current_connection()?.account_id.as_deref() != Some(account) {
            return Ok(None);
        }
        self.provider.receive_health()
    }
    /// One bounded native observation; commits to the existing product store
    /// before acknowledging. A failed ACK leaves durable data for dedup on retry.
    pub fn receive_once_local(&self, account: &str) -> AppResult<ReceiveProgress> {
        let _guard = self
            .receive_guard
            .lock()
            .map_err(|_| AppError::internal())?;
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let batch = self.provider.poll_receive(account)?;
        if batch.context != context {
            return Err(AppError::new("RECEIVE_CONTEXT_CHANGED", "接收身份已经变化"));
        }
        if batch
            .route
            .as_ref()
            .is_some_and(|r| !r.valid_for(&context.sender_id))
            || (batch.message.is_none() && batch.route.is_some())
        {
            return Err(AppError::new(
                "RECEIVE_PROTOCOL_INVALID",
                "接收会话不属于当前身份；未确认消费",
            ));
        }
        let target = batch
            .route
            .as_ref()
            .map_or("filehelper", |r| r.conversation.as_str());
        let epoch = self.send_digest(
            account,
            "receive-epoch",
            &(&context.sender_id, &context.epoch),
        )?;
        let previous = self.receive_cursor_local(account)?;
        let same = previous.as_ref().is_some_and(|c| c.epoch == epoch);
        let mut next = if same {
            previous.clone().ok_or_else(corrupt)?
        } else {
            ReceiveCursor {
                proof: String::new(),
                last_id: None,
                epoch,
                sequence: 0,
                dropped: 0,
                gaps: previous.as_ref().map_or(1, |c| c.gaps.saturating_add(1)),
            }
        };
        // Every new attachment has an unproven pre-attachment interval. A live
        // hook cannot claim historical completeness without a separate backfill.
        if batch.dropped < next.dropped {
            return Err(corrupt());
        }
        if batch.dropped > next.dropped {
            next.gaps = next.gaps.saturating_add(1);
        }
        next.dropped = batch.dropped;
        let protected = if let Some(message) = &batch.message {
            if message.sequence == 0
                || message.sequence < next.sequence
                || !message.valid_content()
                || message.native_message_id.parse::<u64>().is_err()
            {
                return Err(AppError::new(
                    "RECEIVE_PROTOCOL_INVALID",
                    "接收事件格式或顺序无效；未确认消费",
                ));
            }
            if message.sequence > next.sequence.saturating_add(1)
                && batch.dropped == previous.as_ref().filter(|_| same).map_or(0, |c| c.dropped)
            {
                next.gaps = next.gaps.saturating_add(1);
            }
            let id = if message.native_message_id != "0" {
                self.send_digest(
                    account,
                    "receive-native-id",
                    &(&context.sender_id, &target, &message.native_message_id),
                )?
            } else {
                self.send_digest(
                    account,
                    "receive-observation-id",
                    &(&next.epoch, message.sequence),
                )?
            };
            if message.sequence == next.sequence && next.last_id.as_deref() != Some(&id) {
                return Err(AppError::new(
                    "RECEIVE_PROTOCOL_INVALID",
                    "同一观测序列对应不同原生消息；未确认消费",
                ));
            }
            next.sequence = message.sequence;
            next.last_id = Some(id.clone());
            let payload = ReceivedTextPayload {
                schema_version: if batch.route.is_some() {
                    3
                } else if message.message_type == 1 && message.text.len() <= 1023 {
                    1
                } else {
                    2
                },
                account_id: account.into(),
                id: id.clone(),
                sender_id: context.sender_id.clone(),
                target_id: target.to_owned(),
                route: batch.route.clone(),
                observed_at: chrono::Utc::now().to_rfc3339(),
                message: message.clone(),
            };
            // Filehelper IDs/digests remain byte-for-byte compatible with schema1/2.
            // Compare any stored envelope separately so new filehelper route changes
            // cannot be silently accepted as a legacy content-only duplicate.
            if let Some(existing) = self.repository.received_text_by_id(account, &id)? {
                let stored = self.decrypt_received_text(account, existing)?;
                if stored.route.is_some() && stored.route != batch.route {
                    return Err(AppError::new(
                        "RECEIVE_CONTENT_CONFLICT",
                        "同一原生标识的会话信封不同；未确认消费",
                    ));
                }
            }
            let content_digest = if target != "filehelper" {
                self.send_digest(
                    account,
                    "receive-routed-content",
                    &(
                        &context.sender_id,
                        &batch.route,
                        &message.native_message_id,
                        message.message_type,
                        &message.text,
                        &message.metadata,
                    ),
                )?
            } else if message.message_type == 1 {
                self.send_digest(
                    account,
                    "receive-content",
                    &(
                        &context.sender_id,
                        &target,
                        &message.native_message_id,
                        &message.text,
                    ),
                )?
            } else {
                self.send_digest(
                    account,
                    "receive-media-content",
                    &(
                        &context.sender_id,
                        &target,
                        &message.native_message_id,
                        message.message_type,
                        &message.metadata,
                    ),
                )?
            };
            let aad = format!("{}/{}", scope(account, &id), content_digest);
            let mut key = Zeroizing::new([0u8; 32]);
            let mut nonce = [0u8; 12];
            getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
            getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
            let plaintext =
                Zeroizing::new(serde_json::to_vec(&payload).map_err(|_| AppError::internal())?);
            let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
                .map_err(|_| AppError::internal())?
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &plaintext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| AppError::internal())?;
            Some(ProtectedReceivedText {
                account_id: account.into(),
                id,
                content_digest,
                wrapped_key: self.provider.protect_key(&aad, key.as_ref())?,
                nonce: nonce.to_vec(),
                ciphertext,
            })
        } else {
            None
        };
        // Check again after encryption; SQLite also validates current revision
        // in the same transaction that persists the message and cursor.
        if self.provider.resolve_send_context(account, "filehelper")? != context {
            return Err(AppError::new("RECEIVE_CONTEXT_CHANGED", "接收身份已经变化"));
        }
        next.proof = self.send_digest(
            account,
            "receive-cursor",
            &(
                &next.epoch,
                next.sequence,
                next.dropped,
                next.gaps,
                &next.last_id,
            ),
        )?;
        let voice = if let Some(bytes) = &batch.inline_voice {
            let message = batch.message.as_ref().ok_or_else(corrupt)?;
            if message.message_type != 34 || bytes.is_empty() || bytes.len() > 262144 {
                return Err(corrupt());
            }
            let observation_id = protected.as_ref().ok_or_else(corrupt)?.id.clone();
            let file = self.encrypt_file_local(account, "received-voice.silk", bytes.to_vec())?;
            let proof = self.send_digest(
                account,
                "received-voice-link",
                &(&observation_id, &file.asset.id, &file.asset.sha256),
            )?;
            Some(ProtectedReceivedVoice {
                file,
                observation_id,
                proof,
            })
        } else {
            None
        };
        if self.provider.resolve_send_context(account, "filehelper")? != context {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "语音保护期间接收身份已变化",
            ));
        }
        let stored = self.repository.commit_receive(&ReceiveCommit {
            voice: voice.as_ref(),
            account_id: account,
            connection_revision: revision,
            expected: previous.as_ref(),
            next: &next,
            message: protected.as_ref(),
        })?;
        let acknowledged = if let Some(message) = &batch.message {
            self.provider
                .acknowledge_receive(account, &context.epoch, message.sequence)?;
            true
        } else {
            false
        };
        Ok(ReceiveProgress {
            stored,
            acknowledged,
            gaps: next.gaps,
        })
    }
    pub fn received_texts(
        &self,
        principal: &Principal,
        account: &str,
        after: i64,
        limit: u32,
    ) -> AppResult<Vec<ReceivedTextRow>> {
        principal.require("messages:read")?;
        principal.require_account(account)?;
        self.received_texts_local(account, after, limit)
    }
    pub fn receive_cursor_local(&self, account: &str) -> AppResult<Option<ReceiveCursor>> {
        let cursor = self.repository.receive_cursor(account)?;
        if let Some(c) = &cursor
            && self.send_digest(
                account,
                "receive-cursor",
                &(&c.epoch, c.sequence, c.dropped, c.gaps, &c.last_id),
            )? != c.proof
        {
            return Err(corrupt());
        }
        Ok(cursor)
    }
    pub fn received_texts_local(
        &self,
        account: &str,
        after: i64,
        limit: u32,
    ) -> AppResult<Vec<ReceivedTextRow>> {
        self.repository
            .received_texts(account, after, limit)?
            .into_iter()
            .map(|(position, record)| {
                let payload = self.decrypt_received_text(account, record)?;
                let card=payload.message.metadata.as_deref().and_then(|body|werc_domain::message_card::display(payload.message.message_type,body));
                Ok(ReceivedTextRow { position, payload, card })
            })
            .collect()
    }
    pub(crate) fn decrypt_received_text(
        &self,
        account: &str,
        record: ProtectedReceivedText,
    ) -> AppResult<ReceivedTextPayload> {
        if record.account_id != account || record.nonce.len() != 12 {
            return Err(corrupt());
        }
        let aad = format!("{}/{}", scope(account, &record.id), record.content_digest);
        let key = Zeroizing::new(self.provider.unprotect_key(&aad, &record.wrapped_key)?);
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| corrupt())?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&record.nonce),
                    Payload {
                        msg: &record.ciphertext,
                        aad: aad.as_bytes(),
                    },
                )
                .map_err(|_| corrupt())?,
        );
        let payload: ReceivedTextPayload =
            serde_json::from_slice(&plaintext).map_err(|_| corrupt())?;
        if !matches!(payload.schema_version, 1..=3)
            || !payload.message.valid_content()
            || (payload.schema_version == 1
                && (payload.message.message_type != 1 || payload.message.text.len() > 1023))
            || payload.account_id != account
            || payload.id != record.id
            || match (&payload.route, payload.schema_version) {
                (Some(route), 3) => {
                    !route.valid_for(&payload.sender_id) || route.conversation != payload.target_id
                }
                (None, 1 | 2) => payload.target_id != "filehelper",
                _ => true,
            }
        {
            return Err(corrupt());
        }
        Ok(payload)
    }
}
