use crate::SqliteRepository;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use werc_domain::{
    AppError, AppResult,
    media::{AudioAsset, MAX_ACCOUNT_ASSET_BYTES, ProtectedAsset},
};

impl SqliteRepository {
    pub(crate) fn save_asset_store(&self, data: &ProtectedAsset) -> AppResult<()> {
        self.save_asset_checked_store(data, None)
    }
    pub(crate) fn save_asset_checked_store(
        &self,
        data: &ProtectedAsset,
        expected: Option<(i64, i64)>,
    ) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        if let Some(expected) = expected {
            crate::current_store::require_current(&tx, &data.asset.account_id)?;
            let revision: i64 = tx
                .query_row(
                    "SELECT revision FROM current_connection WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| AppError::internal())?;
            let key_revision: i64 = tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')", [&data.asset.account_id], |r| r.get(0)).map_err(|_| AppError::internal())?;
            if (revision, key_revision) != expected {
                return Err(AppError::new(
                    "CURSOR_STALE",
                    "连接或密钥已变化，语音未保存",
                ));
            }
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("DELETE FROM audio_assets WHERE expires_at<=?1", [&now])
            .map_err(|_| AppError::internal())?;
        let (size, count): (i64, i64) = tx
            .query_row(
                "SELECT coalesce(sum(size),0),count(*) FROM audio_assets WHERE account_id=?1",
                [&data.asset.account_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| AppError::internal())?;
        if size + data.asset.size as i64 > MAX_ACCOUNT_ASSET_BYTES as i64 || count >= 128 {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "账号音频配额为 64 MiB / 128 项，请先清理资源",
            ));
        }
        let metadata = serde_json::to_string(&data.asset).map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO audio_assets(id,account_id,metadata,size,wrapped_key,nonce,ciphertext,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![data.asset.id, data.asset.account_id, metadata, data.asset.size as i64, data.wrapped_key, data.nonce, data.ciphertext, data.asset.expires_at]).map_err(|_| AppError::internal())?;
        let event = if expected.is_some() {
            "audio.received"
        } else {
            "audio.uploaded"
        };
        tx.execute(
            "INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,?2,?3,?4)",
            params![
                data.asset.account_id,
                event,
                now,
                serde_json::json!({"asset_id":data.asset.id,"size":data.asset.size,"sent":false})
                    .to_string()
            ],
        )
        .map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn assets_store(&self, account: &str) -> AppResult<Vec<AudioAsset>> {
        let c = self.connection()?;
        let mut s = c.prepare("SELECT metadata FROM audio_assets WHERE account_id=?1 AND expires_at>?2 ORDER BY rowid DESC LIMIT 128").map_err(|_| AppError::internal())?;
        s.query_map(params![account, Utc::now().to_rfc3339()], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| AppError::internal())?
        .map(|r| {
            serde_json::from_str(&r.map_err(|_| AppError::internal())?)
                .map_err(|_| AppError::internal())
        })
        .collect()
    }
    pub(crate) fn asset_store(&self, account: &str, id: &str) -> AppResult<ProtectedAsset> {
        let c = self.connection()?;
        let row = c.query_row("SELECT metadata,wrapped_key,nonce,ciphertext FROM audio_assets WHERE account_id=?1 AND id=?2 AND expires_at>?3", params![account, id, Utc::now().to_rfc3339()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, Vec<u8>>(3)?))).optional().map_err(|_| AppError::internal())?.ok_or_else(|| AppError::new("ASSET_NOT_FOUND", "音频不存在或已过期"))?;
        Ok(ProtectedAsset {
            asset: serde_json::from_str(&row.0).map_err(|_| AppError::internal())?,
            wrapped_key: row.1,
            nonce: row.2,
            ciphertext: row.3,
        })
    }
    pub(crate) fn delete_asset_store(&self, account: &str, id: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        if tx
            .execute(
                "DELETE FROM audio_assets WHERE account_id=?1 AND id=?2",
                params![account, id],
            )
            .map_err(|_| AppError::internal())?
            == 0
        {
            return Err(AppError::new("ASSET_NOT_FOUND", "音频不存在"));
        }
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'audio.removed',?2,?3)", params![account, Utc::now().to_rfc3339(), serde_json::json!({"asset_id":id}).to_string()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub fn purge_expired_audio(&self) -> AppResult<usize> {
        self.connection()?
            .execute(
                "DELETE FROM audio_assets WHERE expires_at<=?1",
                [Utc::now().to_rfc3339()],
            )
            .map_err(|_| AppError::internal())
    }
}
