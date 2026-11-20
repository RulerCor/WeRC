use crate::*;
use rusqlite::TransactionBehavior;
use serde::{Serialize, de::DeserializeOwned};
use werc_domain::{connection::ClientInstallation, outbox::*};

fn json<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|_| AppError::internal())
}
fn decode<T: DeserializeOwned>(value: &str) -> AppResult<T> {
    serde_json::from_str(value).map_err(|_| AppError::new("OUTBOX_CORRUPTED", "发送记录无效"))
}
fn name<T: Serialize>(value: T) -> AppResult<String> {
    serde_json::to_value(value)
        .map_err(|_| AppError::internal())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(AppError::internal)
}
fn load(c: &Connection, account: &str, id: &str) -> AppResult<(SendStatus, ProtectedSend)> {
    let row = c.query_row("SELECT metadata,state,approval_expires_at,attempt_id,evidence,updated_at,wrapped_key,nonce,ciphertext,creator_id,idempotency_digest FROM send_outbox WHERE account_id=?1 AND id=?2", params![account,id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,i64>(5)?,r.get::<_,Vec<u8>>(6)?,r.get::<_,Vec<u8>>(7)?,r.get::<_,Vec<u8>>(8)?,r.get::<_,String>(9)?,r.get::<_,String>(10)?))).optional().map_err(|_|AppError::internal())?.ok_or_else(||AppError::new("OUTBOX_NOT_FOUND","发送任务不存在"))?;
    let metadata: SendMetadata = decode(&row.0)?;
    if metadata.id != id
        || metadata.account_id != account
        || metadata.creator_id != row.9
        || metadata.idempotency_digest != row.10
    {
        return Err(AppError::new("OUTBOX_CORRUPTED", "发送任务隔离信息不一致"));
    }
    let scheduled_at = c
        .query_row(
            "SELECT scheduled_at FROM send_schedules WHERE outbox_id=?1",
            [id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|_| AppError::internal())?;
    let status = SendStatus {
        scheduled_at,
        metadata: metadata.clone(),
        state: decode(&format!("\"{}\"", row.1))?,
        approval_expires_at: row.2,
        attempt_id: row.3,
        evidence: row.4.map(|v| decode(&format!("\"{v}\""))).transpose()?,
        updated_at: row.5,
    };
    Ok((
        status,
        ProtectedSend {
            metadata,
            wrapped_key: row.6,
            nonce: row.7,
            ciphertext: row.8,
        },
    ))
}
fn authorize(c: &Connection, metadata: &SendMetadata) -> AppResult<()> {
    crate::current_store::require_current(c, &metadata.account_id)?;
    let revision: i64 = c
        .query_row(
            "SELECT revision FROM current_connection WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| AppError::internal())?;
    if revision != metadata.connection_revision {
        return Err(AppError::new("CONNECTION_CHANGED", "批准绑定的连接已变化"));
    }
    let installation: Option<String> = c
        .query_row(
            "SELECT installation FROM client_bindings WHERE account_id=?1",
            [&metadata.account_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| AppError::internal())?;
    let installation: ClientInstallation = decode(
        &installation.ok_or_else(|| AppError::new("CONNECTION_CHANGED", "发送连接已解绑"))?,
    )?;
    if installation.sha256 != metadata.installation_sha256 {
        return Err(AppError::new("CONNECTION_CHANGED", "发送客户端版本已变化"));
    }
    if metadata.creator_id != "desktop" {
        let info: Option<String> = c.query_row("SELECT info FROM tokens WHERE id=?1 AND revoked=0 AND (expires_at>?2 OR expires_at=?4) AND (SELECT count(*) FROM token_accounts a WHERE a.token_id=tokens.id)=1 AND EXISTS(SELECT 1 FROM token_accounts a WHERE a.token_id=tokens.id AND a.account_id=?3)",params![metadata.creator_id,Utc::now().to_rfc3339(),metadata.account_id,werc_domain::PERMANENT_TOKEN_EXPIRY],|r|r.get(0)).optional().map_err(|_|AppError::internal())?;
        let info: TokenInfo = decode(
            &info.ok_or_else(|| AppError::new("AUTH_REVOKED", "任务创建令牌已撤销或过期"))?,
        )?;
        if info.id != metadata.creator_id
            || ![
                match metadata.schema_version {
                    2 => "messages:send_voice",
                    3 => "messages:send_image",
                    4 => "messages:send_file",
                    5 => "messages:send_video",
                    6 => "messages:send_emoticon",
                    9 | 10 | 11 | 12 => "moments:publish",
                    _ => "messages:send",
                },
                "operations:create",
            ]
            .iter()
            .all(|s| info.scopes.iter().any(|v| v == s))
        {
            return Err(AppError::new("FORBIDDEN", "任务创建令牌没有发送权限"));
        }
    }
    Ok(())
}
fn event(c: &Connection, status: &SendStatus) -> AppResult<()> {
    c.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'outbox.updated',?2,?3)",params![status.metadata.account_id,Utc::now().to_rfc3339(),serde_json::json!({"outbox_id":status.metadata.id,"state":status.state,"evidence":status.evidence}).to_string()]).map_err(|_|AppError::internal())?;
    Ok(())
}
fn save(c: &Connection, status: &SendStatus) -> AppResult<()> {
    c.execute("UPDATE send_outbox SET state=?1,approval_expires_at=?2,attempt_id=?3,evidence=?4,updated_at=?5 WHERE account_id=?6 AND id=?7",params![name(status.state)?,status.approval_expires_at,status.attempt_id,status.evidence.map(name).transpose()?,status.updated_at,status.metadata.account_id,status.metadata.id]).map_err(|_|AppError::internal())?;
    event(c, status)
}
fn expire_pending(c: &Connection, account: Option<&str>, now: i64) -> AppResult<()> {
    let mut statement=c.prepare("SELECT account_id,id FROM send_outbox WHERE (?1 IS NULL OR account_id=?1) AND attempt_id IS NULL AND state IN ('awaiting_approval','queued') AND (json_extract(metadata,'$.expires_at')<=?2 OR (state='queued' AND approval_expires_at<=?2)) LIMIT 512").map_err(|_|AppError::internal())?;
    let ids = statement
        .query_map(params![account, now], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| AppError::internal())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AppError::internal())?;
    drop(statement);
    for (account, id) in ids {
        let (mut status, _) = load(c, &account, &id)?;
        status.state = SendState::Expired;
        status.updated_at = now;
        save(c, &status)?;
    }
    Ok(())
}
impl SqliteRepository {
    pub(crate) fn send_secret_store(
        &self,
        account: &str,
        candidate: Option<&[u8]>,
    ) -> AppResult<Option<Vec<u8>>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        if let Some(key) = candidate {
            crate::current_store::require_current(&tx, account)?;
            if key.is_empty() || key.len() > 16384 {
                return Err(AppError::new("INVALID_ARGUMENT", "发送摘要保护密钥无效"));
            }
            tx.execute("INSERT INTO send_secrets(account_id,wrapped_key) VALUES(?1,?2) ON CONFLICT(account_id) DO NOTHING",params![account,key]).map_err(|_|AppError::internal())?;
        }
        let key = tx
            .query_row(
                "SELECT wrapped_key FROM send_secrets WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(key)
    }
    pub(crate) fn next_queued_send_store(&self) -> AppResult<Option<SendStatus>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        expire_pending(&tx, None, Utc::now().timestamp())?;
        let row: Option<(String, String)> = tx
            .query_row(
                "SELECT account_id,id FROM send_outbox WHERE state='queued' AND NOT EXISTS(SELECT 1 FROM send_schedules s WHERE s.outbox_id=send_outbox.id AND s.scheduled_at>?1) ORDER BY rowid LIMIT 1",
                [Utc::now().timestamp()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        let result = row
            .map(|(account, id)| load(&tx, &account, &id).map(|v| v.0))
            .transpose()?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(result)
    }
    pub(crate) fn create_send_store(&self, data: &ProtectedSend) -> AppResult<SendStatus> {
        let m = &data.metadata;
        if ![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].contains(&m.schema_version)
            || Uuid::parse_str(&m.id).is_err()
            || data.nonce.len() != 12
            || data.wrapped_key.len() > 16384
            || data.ciphertext.len() > 128 * 1024
            || [
                m.request_digest.as_str(),
                m.context_digest.as_str(),
                m.idempotency_digest.as_str(),
                m.installation_sha256.as_str(),
            ]
            .iter()
            .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(AppError::new("INVALID_ARGUMENT", "受保护发送任务格式无效"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        authorize(&tx, m)?;
        let existing: Option<String> = tx.query_row("SELECT id FROM send_outbox WHERE account_id=?1 AND creator_id=?2 AND idempotency_digest=?3",params![m.account_id,m.creator_id,m.idempotency_digest],|r|r.get(0)).optional().map_err(|_|AppError::internal())?;
        if let Some(id) = existing {
            let (status, _) = load(&tx, &m.account_id, &id)?;
            if status.metadata.request_digest != m.request_digest
                || status.metadata.context_digest != m.context_digest
                || status.metadata.connection_revision != m.connection_revision
            {
                return Err(AppError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "幂等键已经绑定另一内容、身份或连接",
                ));
            }
            return Ok(status);
        }
        let now = Utc::now().timestamp();
        if m.expires_at <= now || m.expires_at > now + 3600 || m.created_at > now + 5 {
            return Err(AppError::new("INVALID_ARGUMENT", "发送草稿期限无效"));
        }
        expire_pending(&tx, Some(&m.account_id), now)?;
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM send_outbox WHERE account_id=?1 AND state IN ('awaiting_approval','queued','submitting')",
                [&m.account_id],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        if count >= 512 {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "尚未处理的发送请求达到512项，请先处理待批准或排队任务；历史与未知结果未删除",
            ));
        }
        tx.execute("INSERT INTO send_outbox(id,account_id,creator_id,idempotency_digest,metadata,state,wrapped_key,nonce,ciphertext,updated_at) VALUES(?1,?2,?3,?4,?5,'awaiting_approval',?6,?7,?8,?9)",params![m.id,m.account_id,m.creator_id,m.idempotency_digest,json(m)?,data.wrapped_key,data.nonce,data.ciphertext,now]).map_err(|_|AppError::internal())?;
        let status = load(&tx, &m.account_id, &m.id)?.0;
        event(&tx, &status)?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(status)
    }
    pub(crate) fn send_status_store(&self, account: &str, id: &str) -> AppResult<SendStatus> {
        let c = self.connection()?;
        Ok(load(&c, account, id)?.0)
    }
    pub(crate) fn send_payload_store(&self, account: &str, id: &str) -> AppResult<ProtectedSend> {
        let c = self.connection()?;
        Ok(load(&c, account, id)?.1)
    }
    pub(crate) fn sends_store(&self, account: &str) -> AppResult<Vec<SendStatus>> {
        let c = self.connection()?;
        // Compact current queue; the history endpoint pages all retained records.
        let mut s=c.prepare("SELECT id FROM send_outbox WHERE account_id=?1 ORDER BY (state IN ('awaiting_approval','queued','submitting')) DESC,updated_at DESC,id LIMIT 512").map_err(|_|AppError::internal())?;
        let ids = s
            .query_map([account], |r| r.get::<_, String>(0))
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::internal())?;
        ids.iter()
            .map(|id| load(&c, account, id).map(|v| v.0))
            .collect()
    }
    pub(crate) fn send_by_idempotency_store(
        &self,
        account: &str,
        creator: &str,
        digest: &str,
    ) -> AppResult<Option<SendStatus>> {
        let c = self.connection()?;
        let id: Option<String> = c.query_row("SELECT id FROM send_outbox WHERE account_id=?1 AND creator_id=?2 AND idempotency_digest=?3", params![account,creator,digest], |r|r.get(0)).optional().map_err(|_|AppError::internal())?;
        id.map(|id| load(&c, account, &id).map(|v| v.0)).transpose()
    }
    pub(crate) fn send_history_page_store(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SendPage> {
        self.send_page_store(account, before, limit, false)
    }
    pub(crate) fn pending_receipt_page_store(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SendPage> {
        self.send_page_store(account, before, limit, true)
    }
    fn send_page_store(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
        pending: bool,
    ) -> AppResult<SendPage> {
        if !(1..=200).contains(&limit) || before.is_some_and(|p| p <= 0) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "历史游标必须为正，页大小1–200",
            ));
        }
        let c = self.connection()?;
        let mut statement=c.prepare("SELECT rowid,id FROM send_outbox WHERE account_id=?1 AND (?2 IS NULL OR rowid<?2) AND (?4=0 OR state IN ('unknown','server_accepted')) ORDER BY rowid DESC LIMIT ?3").map_err(|_|AppError::internal())?;
        let mut ids = statement
            .query_map(params![account, before, limit + 1, pending], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::internal())?;
        let more = ids.len() > limit as usize;
        ids.truncate(limit as usize);
        let next_cursor = if more {
            ids.last().map(|row| row.0)
        } else {
            None
        };
        let data = ids
            .iter()
            .map(|(_, id)| load(&c, account, id).map(|v| v.0))
            .collect::<AppResult<Vec<_>>>()?;
        Ok(SendPage { data, next_cursor })
    }
    pub(crate) fn approve_send_store(
        &self,
        account: &str,
        id: &str,
        digest: &str,
        context: &str,
        until: i64,
        proof: &[u8],
    ) -> AppResult<SendStatus> {
        self.approve_timed_send_store(
            account,
            id,
            digest,
            context,
            &SendTiming {
                scheduled_at: None,
                expires_at: until,
            },
            proof,
        )
    }
    pub(crate) fn approve_timed_send_store(
        &self,
        account: &str,
        id: &str,
        digest: &str,
        context: &str,
        timing: &SendTiming,
        proof: &[u8],
    ) -> AppResult<SendStatus> {
        let until = timing.expires_at;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let (mut status, _) = load(&tx, account, id)?;
        authorize(&tx, &status.metadata)?;
        let now = Utc::now().timestamp();
        if status.state != SendState::AwaitingApproval {
            return Err(AppError::new("INVALID_TRANSITION", "只有待批准任务可批准"));
        }
        if status.metadata.request_digest != digest || status.metadata.context_digest != context {
            return Err(AppError::new("APPROVAL_MISMATCH", "批准摘要已变化"));
        }
        let start = timing.scheduled_at.unwrap_or(now);
        if timing
            .scheduled_at
            .is_some_and(|at| at <= now || at >= status.metadata.expires_at)
            || until <= start
            || until > start.saturating_add(300)
            || until > status.metadata.expires_at
            || proof.is_empty()
            || proof.len() > 16384
        {
            return Err(AppError::new("APPROVAL_EXPIRED", "批准期限无效"));
        }
        if let Some(at) = timing.scheduled_at {
            tx.execute(
                "INSERT INTO send_schedules(outbox_id,scheduled_at) VALUES(?1,?2)",
                params![id, at],
            )
            .map_err(|_| AppError::internal())?;
        }
        status.scheduled_at = timing.scheduled_at;
        status.state = SendState::Queued;
        status.approval_expires_at = Some(until);
        status.updated_at = now;
        tx.execute(
            "UPDATE send_outbox SET approval_proof=?1 WHERE id=?2",
            params![proof, id],
        )
        .map_err(|_| AppError::internal())?;
        save(&tx, &status)?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(status)
    }
    pub(crate) fn send_approval_store(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<Option<Vec<u8>>> {
        self.connection()?
            .query_row(
                "SELECT approval_proof FROM send_outbox WHERE account_id=?1 AND id=?2",
                params![account, id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?
            .ok_or_else(|| AppError::new("OUTBOX_NOT_FOUND", "发送任务不存在"))
    }
    pub(crate) fn start_send_store(
        &self,
        expected: &SendStatus,
        attempt: &str,
    ) -> AppResult<SendStatus> {
        let account = expected.metadata.account_id.as_str();
        let id = expected.metadata.id.as_str();
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let (mut status, _) = load(&tx, account, id)?;
        if status.state != SendState::Queued {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "任务未排队或已提交；不能重复提交",
            ));
        }
        authorize(&tx, &status.metadata)?;
        let now = Utc::now().timestamp();
        if status != *expected {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "预检后任务或批准已变化，没有提交",
            ));
        }
        if status.scheduled_at.is_some_and(|at| at > now) {
            return Err(AppError::new(
                "SEND_NOT_DUE",
                "尚未到计划发送时间，任务保持排队",
            ));
        }
        if status.approval_expires_at.is_none_or(|t| t <= now) || status.metadata.expires_at <= now
        {
            status.state = SendState::Expired;
            status.updated_at = now;
            save(&tx, &status)?;
            tx.commit().map_err(|_| AppError::internal())?;
            return Err(AppError::new(
                "APPROVAL_EXPIRED",
                "发送批准已过期，没有提交",
            ));
        }
        if Uuid::parse_str(attempt).is_err() {
            return Err(AppError::new("INVALID_ARGUMENT", "提交标识无效"));
        }
        let in_flight:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM send_outbox WHERE account_id=?1 AND state='submitting')",[account],|r|r.get(0)).map_err(|_|AppError::internal())?;
        if in_flight {
            return Err(AppError::new("BUSY", "当前微信已有提交尚未退出"));
        }
        tx.execute(
            "INSERT INTO send_attempts(id,outbox_id,started_at) VALUES(?1,?2,?3)",
            params![attempt, id, now],
        )
        .map_err(|_| AppError::internal())?;
        status.state = SendState::Submitting;
        status.attempt_id = Some(attempt.into());
        status.updated_at = now;
        save(&tx, &status)?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(status)
    }
    pub(crate) fn record_send_receipt_store(
        &self,
        account: &str,
        receipt: &ProtectedReceipt,
    ) -> AppResult<SendStatus> {
        if receipt.protected.is_empty() || receipt.protected.len() > 16384 {
            return Err(AppError::new("INVALID_ARGUMENT", "回执保护数据超限"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let (mut status, _) = load(&tx, account, &receipt.outbox_id)?;
        if status.attempt_id.as_deref() != Some(&receipt.attempt_id)
            || status.metadata.context_digest != receipt.context_digest
        {
            return Err(AppError::new(
                "RECEIPT_MISMATCH",
                "回执与提交、原始身份不匹配",
            ));
        }
        let next = status.state.apply_evidence(receipt.evidence)?;
        let changed = next != status.state || status.evidence != Some(receipt.evidence);
        status.state = next;
        status.evidence = Some(receipt.evidence);
        let observed_at = Utc::now().timestamp();
        tx.execute("INSERT INTO send_receipts(outbox_id,attempt_id,evidence,protected,observed_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(outbox_id,attempt_id,evidence) DO UPDATE SET protected=excluded.protected,observed_at=excluded.observed_at",params![receipt.outbox_id,receipt.attempt_id,name(receipt.evidence)?,receipt.protected,observed_at]).map_err(|_|AppError::internal())?;
        if changed {
            status.updated_at = observed_at;
            save(&tx, &status)?;
        }
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(status)
    }
    pub(crate) fn send_receipt_store(&self, account: &str, id: &str) -> AppResult<Option<Vec<u8>>> {
        self.connection()?.query_row("SELECT r.protected FROM send_receipts r JOIN send_outbox o ON o.id=r.outbox_id WHERE o.account_id=?1 AND o.id=?2 ORDER BY r.observed_at DESC,r.rowid DESC LIMIT 1",params![account,id],|r|r.get(0)).optional().map_err(|_|AppError::internal())
    }
    pub(crate) fn cancel_send_store(&self, account: &str, id: &str) -> AppResult<SendStatus> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let (mut status, _) = load(&tx, account, id)?;
        if !matches!(
            status.state,
            SendState::AwaitingApproval | SendState::Queued
        ) {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "已提交或未知任务不能取消或重发",
            ));
        }
        status.state = SendState::Canceled;
        status.updated_at = Utc::now().timestamp();
        save(&tx, &status)?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(status)
    }
    pub(crate) fn mark_send_unknown_store(
        &self,
        account: &str,
        id: &str,
        attempt: &str,
    ) -> AppResult<SendStatus> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let (mut status, _) = load(&tx, account, id)?;
        if status.attempt_id.as_deref() != Some(attempt)
            || !matches!(status.state, SendState::Submitting | SendState::Unknown)
        {
            return Err(AppError::new("INVALID_TRANSITION", "任务已被其他回执确认"));
        }
        status.state = SendState::Unknown;
        status.evidence = Some(SendEvidence::Unknown);
        status.updated_at = Utc::now().timestamp();
        save(&tx, &status)?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(status)
    }
    pub(crate) fn recover_sends_store(&self) -> AppResult<usize> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let mut s = tx
            .prepare("SELECT account_id,id FROM send_outbox WHERE state='submitting'")
            .map_err(|_| AppError::internal())?;
        let ids = s
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::internal())?;
        drop(s);
        for (account, id) in &ids {
            let (mut status, _) = load(&tx, account, id)?;
            status.state = SendState::Unknown;
            status.evidence = Some(SendEvidence::Unknown);
            status.updated_at = Utc::now().timestamp();
            save(&tx, &status)?;
            tx.execute(
                "UPDATE send_attempts SET recovered=1 WHERE outbox_id=?1",
                [id],
            )
            .map_err(|_| AppError::internal())?;
        }
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(ids.len())
    }
}
