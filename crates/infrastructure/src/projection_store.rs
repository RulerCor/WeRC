use crate::*;
use werc_domain::projection::*;

fn missing() -> AppError {
    AppError::new("CURSOR_STALE", "投影已被清理，请重新查询")
}
fn valid_blob(nonce: &[u8], ciphertext: &[u8], limit: usize) -> AppResult<()> {
    if nonce.len() != 12 || ciphertext.len() > limit + 16 {
        return Err(AppError::new("PROJECTION_CORRUPTED", "投影保护格式无效"));
    }
    Ok(())
}
impl SqliteRepository {
    pub(crate) fn projection_store(
        &self,
        account: &str,
        generation: &str,
    ) -> AppResult<Option<ProtectedProjection>> {
        let c = self.connection()?;
        // 在SQLite表达式内限制BLOB，不能先将损坏的大字段复制到Rust再检查。
        c.query_row(
            "SELECT id,
                CASE WHEN typeof(wrapped_key)='blob' AND length(wrapped_key) BETWEEN 1 AND 16384 THEN wrapped_key ELSE x'' END,
                CASE WHEN typeof(nonce)='blob' AND length(nonce)=12 THEN nonce ELSE x'' END,
                CASE WHEN typeof(ciphertext)='blob' AND length(ciphertext) BETWEEN 16 AND ?3 THEN ciphertext ELSE x'' END
             FROM history_projections WHERE account_id=?1 AND generation=?2 AND ready=1",
            params![account, generation, (PROJECTION_HEADER_BYTES + 16) as i64],
            |r| Ok(ProtectedProjection {
                id: r.get(0)?, account_id: account.into(), generation: generation.into(),
                wrapped_key: r.get(1)?, nonce: r.get(2)?, ciphertext: r.get(3)?,
            }),
        ).optional().map_err(|_| AppError::internal())
    }
    pub(crate) fn begin_projection_store(&self, data: &ProtectedProjection) -> AppResult<()> {
        if data.wrapped_key.len() > 16 * 1024 || data.generation.len() != 64 {
            return Err(AppError::new("INVALID_ARGUMENT", "投影保护材料无效"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        // 仅清理超时的未发布缓存；不清理归档，也不删除另一实例仍在构建的暂存。
        tx.execute(
            "DELETE FROM history_projections WHERE account_id=?1 AND ready=0 AND created_at<?2",
            params![data.account_id, Utc::now().timestamp() - 900],
        )
        .map_err(|_| AppError::internal())?;
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM history_projections WHERE account_id=?1 AND ready=0",
                [&data.account_id],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        if count >= 2 {
            return Err(AppError::new("BUSY", "已有投影准备任务，请稍后重试"));
        }
        tx.execute("INSERT INTO history_projections(id,account_id,generation,wrapped_key,nonce,ciphertext,created_at) VALUES(?1,?2,?3,?4,x'',x'',?5)", params![data.id,data.account_id,data.generation,data.wrapped_key,Utc::now().timestamp()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn append_projection_store(
        &self,
        account: &str,
        id: &str,
        rows: &[ProtectedProjectionRow],
    ) -> AppResult<()> {
        if rows.is_empty() || rows.len() > 64 {
            return Err(AppError::new("INVALID_ARGUMENT", "投影写入批次须为1–64项"));
        }
        let mut added = 0usize;
        for row in rows {
            valid_blob(&row.nonce, &row.ciphertext, PROJECTION_ROW_BYTES)?;
            if row.token.len() != 64 {
                return Err(AppError::new("INVALID_ARGUMENT", "投影索引标识无效"));
            }
            added = added
                .checked_add(row.ciphertext.len())
                .ok_or_else(AppError::internal)?;
        }
        if added > PROJECTION_ROW_BYTES + 16 {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "投影写入批次超过16MiB保护预算",
            ));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        let total:i64 = tx.query_row("SELECT COALESCE(SUM(payload_bytes),0) FROM history_projections WHERE account_id=?1", [account], |r| r.get(0)).map_err(|_| AppError::internal())?;
        if total < 0
            || (total as usize)
                .checked_add(added)
                .is_none_or(|n| n > PROJECTION_BYTES)
        {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "投影缓存超过256MiB，未删除历史资料",
            ));
        }
        if tx.execute("UPDATE history_projections SET payload_bytes=payload_bytes+?1 WHERE id=?2 AND account_id=?3 AND ready=0", params![added as i64,id,account]).map_err(|_| AppError::internal())? != 1 { return Err(missing()); }
        {
            let mut statement = tx
                .prepare("INSERT INTO history_projection_rows VALUES(?1,?2,?3,?4)")
                .map_err(|_| AppError::internal())?;
            for row in rows {
                statement
                    .execute(params![id, row.token, row.nonce, row.ciphertext])
                    .map_err(|_| AppError::internal())?;
            }
        }
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn finish_projection_store(&self, data: &ProtectedProjection) -> AppResult<()> {
        valid_blob(&data.nonce, &data.ciphertext, PROJECTION_HEADER_BYTES)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        // 同代已有结果时只丢弃本次暂存，不能发布两份相互覆盖的结果。
        let ready:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM history_projections WHERE account_id=?1 AND generation=?2 AND ready=1)", params![data.account_id,data.generation], |r| r.get(0)).map_err(|_| AppError::internal())?;
        if ready {
            tx.execute(
                "DELETE FROM history_projections WHERE account_id=?1 AND id=?2 AND ready=0",
                params![data.account_id, data.id],
            )
            .map_err(|_| AppError::internal())?;
        } else {
            // 每范围最多保留最近四个读取视图；只淘汰可重建投影，不动来源。
            tx.execute("DELETE FROM history_projections WHERE id IN (SELECT id FROM history_projections WHERE account_id=?1 AND ready=1 ORDER BY created_at DESC,rowid DESC LIMIT -1 OFFSET 3)", [&data.account_id]).map_err(|_| AppError::internal())?;
            let bytes:i64 = tx.query_row("SELECT COALESCE(SUM(payload_bytes),0) FROM history_projections WHERE account_id=?1", [&data.account_id], |r|r.get(0)).map_err(|_| AppError::internal())?;
            if bytes as usize + data.ciphertext.len() > PROJECTION_BYTES {
                return Err(AppError::new("QUOTA_EXCEEDED", "投影缓存超过256MiB"));
            }
            if tx.execute("UPDATE history_projections SET nonce=?1,ciphertext=?2,ready=1,payload_bytes=payload_bytes+?3 WHERE id=?4 AND account_id=?5 AND ready=0", params![data.nonce,data.ciphertext,data.ciphertext.len() as i64,data.id,data.account_id]).map_err(|_| AppError::internal())? != 1 { return Err(missing()); }
        }
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn projection_row_store(
        &self,
        account: &str,
        id: &str,
        token: &str,
    ) -> AppResult<Option<ProtectedProjectionRow>> {
        self.connection()?.query_row(
            "SELECT
                CASE WHEN typeof(r.nonce)='blob' AND length(r.nonce)=12 THEN r.nonce ELSE x'' END,
                CASE WHEN typeof(r.ciphertext)='blob' AND length(r.ciphertext) BETWEEN 16 AND ?4 THEN r.ciphertext ELSE x'' END
             FROM history_projection_rows r JOIN history_projections p ON p.id=r.projection_id
             WHERE p.account_id=?1 AND p.id=?2 AND p.ready=1 AND r.token=?3",
            params![account, id, token, (PROJECTION_ROW_BYTES + 16) as i64],
            |r| Ok(ProtectedProjectionRow { token: token.into(), nonce: r.get(0)?, ciphertext: r.get(1)? }),
        ).optional().map_err(|_| AppError::internal())
    }
    pub(crate) fn clear_projections_store(&self, account: &str) -> AppResult<usize> {
        // 未发布暂存可能属于另一实例，只清理已完成缓存。
        self.connection()?
            .execute(
                "DELETE FROM history_projections WHERE account_id=?1 AND ready=1",
                [account],
            )
            .map_err(|_| AppError::internal())
    }
    pub(crate) fn remove_projection_store(&self, account: &str, id: &str) -> AppResult<()> {
        self.connection()?
            .execute(
                "DELETE FROM history_projections WHERE account_id=?1 AND id=?2",
                params![account, id],
            )
            .map_err(|_| AppError::internal())?;
        Ok(())
    }
}
