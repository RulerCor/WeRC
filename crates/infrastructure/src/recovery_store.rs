use crate::*;
use werc_domain::{live::*, receive::ReceiveCursor, recovery::*};
fn invalid() -> AppError {
    AppError::new("RECOVERY_CORRUPTED", "补读记录格式无效")
}
fn token(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn decode(s: String) -> AppResult<RecoverySummary> {
    serde_json::from_str(&s).map_err(|_| invalid())
}
impl SqliteRepository {
    pub(crate) fn commit_recovery_store(
        &self,
        data: &RecoveryCommit<'_>,
    ) -> AppResult<RecoverySummary> {
        let r = data.record;
        let s = &r.summary;
        if r.account_id != data.account_id
            || !token(&s.id)
            || !token(&s.proof)
            || s.rows > LIVE_WINDOW_MAX_ROWS
            || s.text_rows > s.rows
            || s.captured_at.len() > 64
            || r.nonce.len() != 12
            || r.wrapped_key.is_empty()
            || r.wrapped_key.len() > 16384
            || !(16..=LIVE_WINDOW_MAX_BYTES + 16400).contains(&r.ciphertext.len())
        {
            return Err(invalid());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, data.account_id)?;
        let revision: i64 = tx
            .query_row(
                "SELECT revision FROM current_connection WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        let key_revision:i64=tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')",[data.account_id],|r|r.get(0)).map_err(|_|AppError::internal())?;
        let cursor: Option<String> = tx
            .query_row(
                "SELECT cursor FROM receive_streams WHERE account_id=?1",
                [data.account_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        let cursor: Option<ReceiveCursor> = cursor
            .map(|s| serde_json::from_str(&s).map_err(|_| invalid()))
            .transpose()?;
        if revision != data.connection_revision
            || key_revision != data.key_revision
            || cursor.as_ref() != data.expected_receive
        {
            return Err(AppError::new(
                "RECOVERY_CONTEXT_CHANGED",
                "连接、密钥或接收位置已变化；未保存补读记录",
            ));
        }
        let existing: Option<String> = tx
            .query_row(
                "SELECT summary FROM receive_recovery_windows WHERE account_id=?1 AND id=?2",
                params![data.account_id, s.id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| AppError::internal())?;
        if let Some(existing) = existing {
            return decode(existing);
        }
        let (count,bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(ciphertext)+length(wrapped_key)),0) FROM receive_recovery_windows WHERE account_id=?1",[data.account_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|AppError::internal())?;
        if count >= MAX_RECOVERY_WINDOWS as i64
            || bytes as usize + r.ciphertext.len() + r.wrapped_key.len() > MAX_RECOVERY_BYTES
        {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "补读记录已达128个窗口或256MiB上限；未删除旧记录",
            ));
        }
        tx.execute("INSERT INTO receive_recovery_windows(account_id,id,summary,wrapped_key,nonce,ciphertext) VALUES(?1,?2,?3,?4,?5,?6)",params![data.account_id,s.id,serde_json::to_string(s).map_err(|_|invalid())?,r.wrapped_key,r.nonce,r.ciphertext]).map_err(|_|AppError::internal())?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'receive.source_saved',?2,?3)",params![data.account_id,s.captured_at,serde_json::json!({"rows":s.rows,"text_rows":s.text_rows,"gaps_closed":false}).to_string()]).map_err(|_|AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())?;
        Ok(s.clone())
    }
    pub(crate) fn recovery_summaries_store(
        &self,
        account: &str,
    ) -> AppResult<Vec<RecoverySummary>> {
        let c = self.connection()?;
        let mut q=c.prepare("SELECT summary FROM receive_recovery_windows WHERE account_id=?1 ORDER BY position DESC LIMIT 129").map_err(|_|AppError::internal())?;
        let rows = q
            .query_map([account], |r| r.get::<_, String>(0))
            .map_err(|_| AppError::internal())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AppError::internal())?;
        if rows.len() > MAX_RECOVERY_WINDOWS {
            return Err(invalid());
        }
        rows.into_iter().map(decode).collect()
    }
    pub(crate) fn recovery_store(&self, account: &str, id: &str) -> AppResult<ProtectedRecovery> {
        if !token(id) {
            return Err(AppError::new("INVALID_ARGUMENT", "补读记录标识无效"));
        }
        let c = self.connection()?;
        let (summary,wrapped_key,nonce,ciphertext):(String,Vec<u8>,Vec<u8>,Vec<u8>)=c.query_row(
            "SELECT summary,wrapped_key,nonce,ciphertext FROM receive_recovery_windows WHERE account_id=?1 AND id=?2 AND length(summary)<=4096 AND length(wrapped_key)<=16384 AND length(nonce)=12 AND length(ciphertext)<=2113552",
            params![account,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|AppError::internal())?.ok_or_else(||AppError::new("NOT_FOUND","补读记录不存在或格式无效"))?;
        Ok(ProtectedRecovery {
            account_id: account.into(),
            summary: decode(summary)?,
            wrapped_key,
            nonce,
            ciphertext,
        })
    }
}
