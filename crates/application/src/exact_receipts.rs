//! Durable, account/source-bound reconciliation of a causally captured native ID.
//! This path only reads existing rows; it cannot submit or renew a send approval.
use crate::Application;
use serde::{Deserialize, Serialize};
use werc_domain::{AppError, AppResult, library::DocumentConnection, live::*, outbox::*};

use crate::backfill::BoundSource;
#[derive(Serialize, Deserialize)]
struct ExactMessageProof {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_record_observed_at: Option<i64>,
    attempt: String,
    context_digest: String,
    login: String,
    identity: NativeMessageIdentity,
    sources: Vec<BoundSource>,
}
fn document_id(id: &str) -> String {
    format!("send-exact-{id}")
}
fn message_type(payload: &TextSendPayload) -> Option<u64> {
    if payload.moment.is_some() || payload.context.execution_mode != SendExecutionMode::Native {
        return None;
    }
    Some(if payload.quote.is_some() {
        (57u64 << 32) | 49
    } else if payload.file.is_some() {
        (6u64 << 32) | 49
    } else if payload.voice.is_some() {
        34
    } else if payload.image.is_some() {
        3
    } else if payload.video.is_some() {
        43
    } else if payload.emoticon.is_some() {
        47
    } else {
        1
    })
}

fn mismatch() -> AppError {
    AppError::new("RECEIPT_MISMATCH", "精确回执与原提交不符，保留原结果")
}

/// Require the causal local ID plus independent row attributes. The body is an
/// integrity check, never the mechanism that selects a message.
fn verify_row(
    identity: &NativeMessageIdentity,
    payload: &TextSendPayload,
    row: &LiveMessage,
) -> AppResult<Option<String>> {
    if row.local_id != identity.local_id.to_string() {
        return Ok(None);
    }
    if row.timestamp != identity.timestamp
        || row.message_type.parse::<u64>().ok() != Some(identity.message_type)
        || message_type(payload) != Some(identity.message_type)
        || row.sender_username.as_deref() != Some(payload.context.sender_id.as_str())
    {
        return Err(mismatch());
    }
    if identity.message_type == 1 && row.text.as_deref() != Some(payload.text.as_str()) {
        return Err(mismatch());
    }
    if !payload.mentions.is_empty() {
        let expected: std::collections::BTreeSet<_> = payload.mentions.iter().collect();
        let observed = row
            .at_user_list
            .as_ref()
            .map(|members| members.iter().collect::<std::collections::BTreeSet<_>>());
        if observed.as_ref() != Some(&expected) {
            return Err(mismatch());
        }
    }
    if let Some(quote) = &payload.quote {
        let xml = row.metadata_body().ok_or_else(mismatch)?;
        if !werc_domain::message_card::quote_matches(
            xml,
            &payload.text,
            &quote.server_id,
            &quote.text,
        )
        .map_err(|_| mismatch())?
        {
            return Err(mismatch());
        }
    }
    Ok(row
        .server_id
        .as_ref()
        .filter(|s| {
            s.parse::<i64>()
                .is_ok_and(|n| n != 0 && n.to_string() == **s)
        })
        .cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_receipt_requires_the_submitted_members_even_when_text_and_id_match() {
        let payload: TextSendPayload = serde_json::from_value(serde_json::json!({
            "context": {"sender_id":"owned", "target_id":"owned@chatroom", "target_display_name":"owned group",
                "identity_verified":true, "target_verified":true, "epoch":"owned", "installation_sha256":"owned",
                "adapter_sha256":"owned", "execution_mode":"native"},
            "text":"自动化测试：成员提醒", "mentions":["first", "second"]
        })).unwrap();
        let identity = NativeMessageIdentity {
            local_id: 487,
            timestamp: 1700000000,
            message_type: 1,
        };
        let mut row = LiveMessage {
            local_id: "487".into(),
            timestamp: 1700000000,
            server_id: Some("12345".into()),
            message_type: "1".into(),
            text: Some(payload.text.clone()),
            metadata: None,
            content_status: "text".into(),
            sender_username: Some("owned".into()),
            at_user_list: None,
        };
        for members in [
            None,
            Some(vec![]),
            Some(vec!["first".into()]),
            Some(vec!["first".into(), "second".into(), "third".into()]),
            Some(vec!["notify@all".into()]),
        ] {
            row.at_user_list = members;
            assert_eq!(
                verify_row(&identity, &payload, &row).unwrap_err().code,
                "RECEIPT_MISMATCH"
            );
        }
        row.at_user_list = Some(vec!["second".into(), "first".into()]);
        assert_eq!(
            verify_row(&identity, &payload, &row).unwrap(),
            Some("12345".into())
        );
        row.local_id = "488".into();
        assert_eq!(verify_row(&identity, &payload, &row).unwrap(), None);
    }
}

impl Application {
    pub(crate) fn send_local_record(
        &self,
        status: &SendStatus,
        payload: &TextSendPayload,
    ) -> AppResult<Option<LocalRecordStatus>> {
        if message_type(payload).is_none() || status.attempt_id.is_none() {
            return Ok(None);
        }
        let observed = match self.open_document::<ExactMessageProof>(
            &status.metadata.account_id,
            &document_id(&status.metadata.id),
            "send_exact",
        ) {
            Ok((_, proof)) => {
                if Some(proof.attempt.as_str()) != status.attempt_id.as_deref()
                    || proof.context_digest != status.metadata.context_digest
                    || proof.login != payload.context.sender_id
                    || message_type(payload) != Some(proof.identity.message_type)
                {
                    return Err(mismatch());
                }
                proof.local_record_observed_at
            }
            Err(e) if e.code == "NOT_FOUND" => None,
            Err(e) => return Err(e),
        };
        Ok(Some(LocalRecordStatus {
            state: if observed.is_some() {
                "written"
            } else {
                "unknown"
            },
            observed_at: observed,
        }))
    }
    /// Called only while the original submission context is still authenticated.
    /// Persist the identity before attempting any source read so a temporary read
    /// failure does not lose the ability to reconcile after reconnecting.
    pub(crate) fn remember_exact_receipt(
        &self,
        status: &SendStatus,
        payload: &TextSendPayload,
        receipt: &SendReceipt,
    ) -> AppResult<()> {
        if message_type(payload).is_none() {
            return Ok(());
        }
        let Some(identity) = receipt
            .provider_reference
            .as_deref()
            .and_then(NativeMessageIdentity::parse)
        else {
            return Ok(());
        };
        if message_type(payload) != Some(identity.message_type) {
            return Err(mismatch());
        }
        let account = &status.metadata.account_id;
        let id = document_id(&status.metadata.id);
        let revision = match self.open_document::<ExactMessageProof>(account, &id, "send_exact") {
            Ok((_, old)) => {
                if old.identity != identity
                    || old.attempt != receipt.attempt_id
                    || old.context_digest != receipt.context_digest
                    || old.login != payload.context.sender_id
                {
                    return Err(mismatch());
                }
                return Ok(());
            }
            Err(e) if e.code == "NOT_FOUND" => 1,
            Err(e) => return Err(e),
        };
        let sources = self.enrolled_message_sources(account, &payload.context.sender_id)?;
        if sources.is_empty() {
            return Ok(());
        }
        // Do not freeze a changed file identity into a cross-connection proof.
        for source in &sources {
            let (_, salt, fingerprint) = self.live_read_binding(account, &source.database)?;
            if salt != source.salt || fingerprint != source.fingerprint {
                return Err(AppError::new("LIVE_SOURCE_REPLACED", "回执来源已经变化"));
            }
        }
        let proof = ExactMessageProof {
            local_record_observed_at: None,
            attempt: receipt.attempt_id.clone(),
            context_digest: receipt.context_digest.clone(),
            login: payload.context.sender_id.clone(),
            identity,
            sources,
        };
        let sealed = self.seal_document(account, &id, "send_exact", revision, &proof)?;
        self.repository.library_save(
            &[sealed],
            Some(&DocumentConnection {
                revision: status.metadata.connection_revision,
                key_revision: self.repository.key_revision(account)?,
            }),
        )
    }

    /// Returns None only when no durable native identity is available. An existing
    /// proof can yield Unknown; that is not permission to use a new send attempt.
    pub(crate) fn reconcile_exact_receipt(
        &self,
        status: &SendStatus,
        payload: &TextSendPayload,
    ) -> AppResult<Option<SendReceipt>> {
        if message_type(payload).is_none() {
            return Ok(None);
        }
        let account = &status.metadata.account_id;
        let (header, mut proof) = match self.open_document::<ExactMessageProof>(
            account,
            &document_id(&status.metadata.id),
            "send_exact",
        ) {
            Ok(value) => value,
            Err(e) if e.code == "NOT_FOUND" => return Ok(None),
            Err(e) => return Err(e),
        };
        if Some(proof.attempt.as_str()) != status.attempt_id.as_deref()
            || proof.context_digest != status.metadata.context_digest
            || proof.login != payload.context.sender_id
            || proof.sources.is_empty()
            || NativeMessageIdentity::parse(&proof.identity.reference()).is_none()
            || message_type(payload) != Some(proof.identity.message_type)
        {
            return Err(mismatch());
        }
        let _history = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源读取正在执行"))?;
        // A fresh filehelper context authenticates the current login without
        // preparing an expired send target or reusing the old connection epoch.
        let current = self.checked_send_context(account, "filehelper")?;
        if current.0.sender_id != proof.login
            || current.0.execution_mode != SendExecutionMode::Native
        {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "当前登录者与原提交不同",
            ));
        }
        let keys = self.live_keys_local(account)?;
        let lease = self.browse_lease("exact-send-receipt")?;
        let mut found: Option<(String, String)> = None;
        let mut matching_rows = 0;
        for source in &proof.sources {
            lease.check()?;
            let (binding, salt, fingerprint) = self.live_read_binding(account, &source.database)?;
            if salt != source.salt || fingerprint != source.fingerprint {
                return Err(AppError::new("LIVE_SOURCE_REPLACED", "原回执来源已被替换"));
            }
            let key = crate::keyring::key_for(&keys, &salt)?;
            let mut query = LiveMessageQuery::default();
            query.from_timestamp = Some(proof.identity.timestamp);
            query.until_timestamp = Some(proof.identity.timestamp + 1);
            let mut selection = LiveConversationSelection {
                username: payload.context.target_id.clone(),
                before: None,
                include_anchor: false,
                newer_context: false,
                query: Some(query),
            };
            loop {
                let window = match self.provider.read_live_conversation(
                    account,
                    &source.database,
                    &key,
                    &fingerprint,
                    &selection,
                    &lease.cancel,
                ) {
                    Ok(window) => window,
                    Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => break,
                    Err(e) => return Err(e),
                };
                let _validated = crate::live_view::encode_window(&window)?;
                lease.check()?;
                if self.live_read_binding(account, &source.database)?.0 != binding {
                    return Err(AppError::new(
                        "CONNECTION_CHANGED",
                        "回查期间来源或登录身份变化",
                    ));
                }
                for row in &window.rows {
                    if row.timestamp != proof.identity.timestamp {
                        return Err(mismatch());
                    }
                    if row.local_id == proof.identity.local_id.to_string() {
                        matching_rows += 1;
                        if matching_rows > 1 {
                            return Err(AppError::new(
                                "RECEIPT_AMBIGUOUS",
                                "多个来源具有相同本地编号，原结果保留",
                            ));
                        }
                    }
                    if let Some(server) = verify_row(&proof.identity, payload, row)? {
                        if found.is_some() {
                            return Err(AppError::new(
                                "RECEIPT_AMBIGUOUS",
                                "多个来源具有相同本地编号，原结果保留",
                            ));
                        }
                        found = Some((fingerprint.clone(), server));
                    }
                }
                if !window.has_older {
                    break;
                }
                let last = window.rows.last().ok_or_else(mismatch)?;
                let next = LiveAnchor {
                    timestamp: last.timestamp,
                    local_id: last.local_id.clone(),
                    server_id: last.server_id.clone(),
                };
                if selection
                    .before
                    .as_ref()
                    .is_some_and(|old| next.key() >= old.key())
                {
                    return Err(mismatch());
                }
                selection.before = Some(next);
            }
        }
        if self.checked_send_context(account, "filehelper")? != current {
            return Err(AppError::new("CONNECTION_CHANGED", "回查期间登录变化"));
        }
        // A verified local row establishes a local write even if server_id is
        // still zero. Keep that fact across deletion/reconnect independently of
        // the delivery state. A missing row never proves a write failure.
        if matching_rows == 1 && proof.local_record_observed_at.is_none() {
            proof.local_record_observed_at = Some(chrono::Utc::now().timestamp());
            let document = self.seal_document(
                account,
                &header.id,
                "send_exact",
                header.revision + 1,
                &proof,
            )?;
            self.repository.library_save(
                &[document],
                Some(&DocumentConnection {
                    revision: current.1,
                    key_revision: self.repository.key_revision(account)?,
                }),
            )?;
        }
        let accepted = found.is_some();
        Ok(Some(SendReceipt {
            outbox_id: status.metadata.id.clone(),
            attempt_id: proof.attempt,
            context_digest: proof.context_digest,
            evidence: if accepted {
                SendEvidence::ServerAccepted
            } else {
                SendEvidence::Unknown
            },
            // All identifiers remain inside the existing encrypted receipt.
            provider_reference: Some(match found {
                Some((source, server)) => format!("native-text-server:v1:{source}/{server}"),
                None => proof.identity.reference(),
            }),
        }))
    }
}
