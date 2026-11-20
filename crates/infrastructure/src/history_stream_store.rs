use crate::SqliteRepository;
use chrono::{Duration, Utc};
use rusqlite::{OptionalExtension, params};
use werc_domain::{AppError, AppResult, history::*};

const ACCOUNT_BYTES: i64 = 8 * 1024 * 1024 * 1024;

impl SqliteRepository {
    pub(crate) fn begin_history_store(&self, data: &ProtectedHistory) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let cutoff = (Utc::now() - Duration::hours(24)).to_rfc3339();
        tx.execute(
            "DELETE FROM history_sources WHERE visible=0 AND started_at<?1",
            [&cutoff],
        )
        .map_err(|_| AppError::internal())?;
        let count: i64 = tx
            .query_row(
                "SELECT count(*) FROM history_sources WHERE account_id=?1",
                [&data.source.account_id],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        if count >= 512 {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "账号来源达到512份预算；未忽略剩余来源",
            ));
        }
        tx.execute("INSERT INTO history_sources(id,account_id,source_sha256,metadata,wrapped_key,nonce,ciphertext,storage_version,visible,started_at) VALUES(?1,?2,?3,?4,?5,zeroblob(12),X'',2,0,?6)", params![data.source.id,data.source.account_id,format!("pending:{}",data.source.id),serde_json::to_string(&data.source).map_err(|_| AppError::internal())?,data.wrapped_key,Utc::now().to_rfc3339()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn append_history_batch_store(
        &self,
        account: &str,
        id: &str,
        batch: &ProtectedHistoryBatch,
    ) -> AppResult<()> {
        if batch.nonce.len() != 12
            || batch.ciphertext.len() > HISTORY_STORED_BATCH_BYTES + 16
            || batch.sequence >= 1_000_000
        {
            return Err(AppError::new(
                "READER_PROTOCOL_INVALID",
                "加密批次超出受限契约",
            ));
        }
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let expected: Option<u64> = tx.query_row("SELECT batch_count FROM history_sources WHERE account_id=?1 AND id=?2 AND visible=0 AND storage_version=2", params![account,id], |r| r.get(0)).optional().map_err(|_| AppError::internal())?;
        if expected != Some(batch.sequence) {
            return Err(AppError::new("SOURCE_CONFLICT", "导入批次顺序或来源已变化"));
        }
        let size: i64 = tx.query_row("SELECT coalesce(sum(payload_bytes+length(ciphertext)),0) FROM history_sources WHERE account_id=?1", [account], |r| r.get(0)).map_err(|_| AppError::internal())?;
        if size + batch.ciphertext.len() as i64 > ACCOUNT_BYTES {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "账号加密历史达到8GiB存储预算",
            ));
        }
        tx.execute(
            "INSERT INTO history_batches(source_id,sequence,nonce,ciphertext) VALUES(?1,?2,?3,?4)",
            params![id, batch.sequence, batch.nonce, batch.ciphertext],
        )
        .map_err(|_| AppError::internal())?;
        for record in &batch.records {
            tx.execute("INSERT INTO history_record_digests(source_id,message_id,digest) VALUES(?1,?2,?3) ON CONFLICT(source_id,message_id) DO UPDATE SET digest=excluded.digest", params![id,record.message_id,record.digest]).map_err(|_| AppError::internal())?;
        }
        for change in &batch.changes {
            tx.execute("INSERT INTO history_pending_events(source_id,message_id,change) VALUES(?1,?2,?3) ON CONFLICT(source_id,message_id) DO UPDATE SET change=excluded.change", params![id,change.message_id,change.change]).map_err(|_| AppError::internal())?;
        }
        tx.execute("UPDATE history_sources SET batch_count=batch_count+1,payload_bytes=payload_bytes+?1 WHERE id=?2", params![batch.ciphertext.len() as i64,id]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn finish_history_store(
        &self,
        data: &ProtectedHistory,
        expected_revision: i64,
    ) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, &data.source.account_id)?;
        let revision:i64=tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')",[&data.source.account_id],|r|r.get(0)).map_err(|_|AppError::internal())?;
        if revision != expected_revision {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "提交期间密钥或来源授权已改变",
            ));
        }
        let changed = tx.execute("UPDATE history_sources SET metadata=?1,source_sha256=?2,nonce=?3,ciphertext=?4,visible=1 WHERE account_id=?5 AND id=?6 AND visible=0", params![serde_json::to_string(&data.source).map_err(|_| AppError::internal())?,data.source.source_sha256,data.nonce,data.ciphertext,data.source.account_id,data.source.id]).map_err(|_| AppError::new("SOURCE_CONFLICT", "此来源已完成或相同快照已由另一导入提交"))?;
        if changed != 1 {
            return Err(AppError::new("SOURCE_CONFLICT", "导入来源已改变"));
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'history.imported',?2,?3)", params![data.source.account_id,now,serde_json::json!({"source_id":data.source.id,"message_count":data.source.message_count,"data_mode":"imported_snapshot"}).to_string()]).map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) SELECT ?1,'history.message_observed',?2,json_object('source_id',source_id,'message_id',message_id,'change',change,'data_mode','imported_snapshot') FROM history_pending_events WHERE source_id=?3 ORDER BY message_id", params![data.source.account_id,now,data.source.id]).map_err(|_| AppError::internal())?;
        tx.execute(
            "DELETE FROM history_pending_events WHERE source_id=?1",
            [&data.source.id],
        )
        .map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn abort_history_store(&self, account: &str, id: &str) -> AppResult<()> {
        self.connection()?
            .execute(
                "DELETE FROM history_sources WHERE account_id=?1 AND id=?2 AND visible=0",
                params![account, id],
            )
            .map_err(|_| AppError::internal())?;
        Ok(())
    }
    pub(crate) fn history_batch_store(
        &self,
        account: &str,
        id: &str,
        sequence: u64,
    ) -> AppResult<Option<ProtectedHistoryBatch>> {
        self.connection()?.query_row("SELECT b.nonce,b.ciphertext FROM history_batches b JOIN history_sources s ON s.id=b.source_id WHERE s.account_id=?1 AND s.id=?2 AND s.visible=1 AND b.sequence=?3", params![account,id,sequence], |r| Ok(ProtectedHistoryBatch { sequence, nonce:r.get(0)?,ciphertext:r.get(1)?,records:Vec::new(),changes:Vec::new() })).optional().map_err(|_| AppError::internal())
    }
    pub(crate) fn history_index_digest_store(
        &self,
        account: &str,
        source: &str,
        check: &dyn Fn() -> AppResult<()>,
    ) -> AppResult<String> {
        use sha2::{Digest, Sha256};
        let c = self.connection()?;
        let mut statement=c.prepare("SELECT d.message_id,d.digest FROM history_record_digests d JOIN history_sources s ON s.id=d.source_id WHERE s.account_id=?1 AND s.id=?2 ORDER BY d.message_id").map_err(|_| AppError::internal())?;
        let mut rows = statement
            .query(params![account, source])
            .map_err(|_| AppError::internal())?;
        let mut hash = Sha256::new();
        let mut count = 0usize;
        while let Some(row) = rows.next().map_err(|_| AppError::internal())? {
            if count.is_multiple_of(500) {
                check()?;
            }
            count += 1;
            let id: String = row.get(0).map_err(|_| AppError::internal())?;
            let digest: String = row.get(1).map_err(|_| AppError::internal())?;
            if id.len() != 64 || digest.len() != 64 {
                return Err(AppError::new("SOURCE_CORRUPTED", "消息摘要索引格式无效"));
            }
            hash.update(id.as_bytes());
            hash.update(digest.as_bytes());
        }
        Ok(hex::encode(hash.finalize()))
    }
    pub(crate) fn history_message_states_store(
        &self,
        account: &str,
        ids: &[String],
    ) -> AppResult<std::collections::BTreeMap<String, HistoryMessageState>> {
        if ids.len() > HISTORY_BATCH_ROWS
            || ids
                .iter()
                .any(|id| id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "消息状态查询最多允许500个64位十六进制ID",
            ));
        }
        if ids.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        let encoded = serde_json::to_string(ids).map_err(|_| AppError::internal())?;
        let c = self.connection()?;
        let mut statement = c.prepare(
            "WITH requested AS (SELECT DISTINCT value AS message_id FROM json_each(?2)),
             ranked AS (
                 SELECT d.message_id,d.source_id,d.digest,
                        row_number() OVER (PARTITION BY d.message_id ORDER BY s.rowid DESC) AS position
                 FROM requested r
                 JOIN history_record_digests d ON d.message_id=r.message_id
                 JOIN history_sources s ON s.id=d.source_id
                 WHERE s.account_id=?1 AND s.visible=1
             )
             SELECT message_id,source_id,digest FROM ranked WHERE position=1",
        ).map_err(|_| AppError::internal())?;
        let rows = statement
            .query_map(params![account, encoded], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    HistoryMessageState {
                        source_id: row.get(1)?,
                        digest: row.get(2)?,
                    },
                ))
            })
            .map_err(|_| AppError::internal())?;
        rows.map(|row| row.map_err(|_| AppError::internal()))
            .collect()
    }
    pub(crate) fn history_message_digest_store(
        &self,
        account: &str,
        message_id: &str,
    ) -> AppResult<Option<String>> {
        self.connection()?.query_row("SELECT d.digest FROM history_record_digests d JOIN history_sources s ON s.id=d.source_id WHERE s.account_id=?1 AND s.visible=1 AND d.message_id=?2 ORDER BY s.rowid DESC LIMIT 1", params![account,message_id], |r| r.get(0)).optional().map_err(|_| AppError::internal())
    }
    pub(crate) fn history_message_is_current_store(
        &self,
        account: &str,
        source: &str,
        message_id: &str,
    ) -> AppResult<bool> {
        let found: Option<String> = self.connection()?.query_row("SELECT d.source_id FROM history_record_digests d JOIN history_sources s ON s.id=d.source_id WHERE s.account_id=?1 AND s.visible=1 AND d.message_id=?2 ORDER BY s.rowid DESC LIMIT 1", params![account,message_id], |r| r.get(0)).optional().map_err(|_| AppError::internal())?;
        Ok(found.is_none_or(|id| id == source))
    }
}
