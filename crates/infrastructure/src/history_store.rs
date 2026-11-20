use crate::SqliteRepository;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use werc_domain::{
    AppError, AppResult,
    history::{HistorySource, ProtectedHistory},
};
impl SqliteRepository {
    pub(crate) fn history_sources_store(&self, account: &str) -> AppResult<Vec<HistorySource>> {
        let c = self.connection()?;
        let mut s = c
            .prepare("SELECT metadata FROM history_sources WHERE account_id=?1 AND visible=1 ORDER BY id")
            .map_err(|_| AppError::internal())?;
        s.query_map([account], |r| r.get::<_, String>(0))
            .map_err(|_| AppError::internal())?
            .map(|r| {
                serde_json::from_str(&r.map_err(|_| AppError::internal())?)
                    .map_err(|_| AppError::internal())
            })
            .collect()
    }
    pub(crate) fn history_payload_store(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<ProtectedHistory> {
        let c = self.connection()?;
        let r = c.query_row("SELECT metadata,wrapped_key,nonce,ciphertext,storage_version FROM history_sources WHERE account_id=?1 AND id=?2 AND visible=1", params![account,id], |r| Ok((r.get::<_,String>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get::<_,u32>(4)?)))
            .optional().map_err(|_| AppError::internal())?.ok_or_else(|| AppError::new("SOURCE_NOT_FOUND", "导入来源不存在"))?;
        Ok(ProtectedHistory {
            version: r.4,
            changes: Vec::new(),
            source: serde_json::from_str(&r.0).map_err(|_| AppError::internal())?,
            wrapped_key: r.1,
            nonce: r.2,
            ciphertext: r.3,
        })
    }
    pub(crate) fn save_history_store(&self, data: &ProtectedHistory) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let (count,size): (i64,i64) = tx.query_row("SELECT count(*),coalesce(sum(length(ciphertext)),0) FROM history_sources WHERE account_id=?1", [&data.source.account_id], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|_| AppError::internal())?;
        if count >= 16 || size + data.ciphertext.len() as i64 > 64 * 1024 * 1024 {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "账号导入配额为16份/64MiB加密结果，请先清理旧来源",
            ));
        }
        tx.execute("INSERT INTO history_sources(id,account_id,source_sha256,metadata,wrapped_key,nonce,ciphertext) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![data.source.id,data.source.account_id,data.source.source_sha256,serde_json::to_string(&data.source).map_err(|_| AppError::internal())?,data.wrapped_key,data.nonce,data.ciphertext]).map_err(|_| AppError::new("SOURCE_CONFLICT", "同一来源已导入或账号已移除，请刷新列表"))?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'history.imported',?2,?3)",params![data.source.account_id,Utc::now().to_rfc3339(),serde_json::json!({"source_id":data.source.id,"message_count":data.source.message_count,"data_mode":"imported_snapshot"}).to_string()]).map_err(|_| AppError::internal())?;
        for change in &data.changes {
            tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'history.message_observed',?2,?3)",params![data.source.account_id,Utc::now().to_rfc3339(),serde_json::json!({"source_id":data.source.id,"message_id":change.message_id,"change":change.change,"data_mode":"imported_snapshot"}).to_string()]).map_err(|_|AppError::internal())?;
        }
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn remove_history_store(&self, account: &str, id: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        if tx
            .execute(
                "DELETE FROM history_sources WHERE account_id=?1 AND id=?2",
                params![account, id],
            )
            .map_err(|_| AppError::internal())?
            == 0
        {
            return Err(AppError::new("SOURCE_NOT_FOUND", "导入来源不存在"));
        }
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'history.removed',?2,?3)",params![account,Utc::now().to_rfc3339(),serde_json::json!({"source_id":id}).to_string()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
}
