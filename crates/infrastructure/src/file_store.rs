use crate::SqliteRepository;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use werc_domain::{
    AppError, AppResult,
    file::{FileAsset, MAX_FILE_BYTES, ProtectedFile},
    media::MAX_ACCOUNT_ASSET_BYTES,
};

impl SqliteRepository {
    pub(crate) fn save_file_store(&self, data: &ProtectedFile) -> AppResult<()> {
        self.save_file_store_inner(data, None)
    }
    pub(crate) fn received_voice_link_store(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>> {
        self.connection()?.query_row("SELECT CASE WHEN length(r.asset_id)<=64 THEN r.asset_id END,CASE WHEN length(r.proof)=64 THEN r.proof END FROM received_voice_assets r JOIN file_assets f ON f.id=r.asset_id AND f.account_id=r.account_id WHERE r.account_id=?1 AND r.observation_id=?2 AND f.expires_at>?3",params![account,observation_id,Utc::now().to_rfc3339()],|r|Ok(werc_domain::received_media::ReceivedImageLink {asset_id:r.get(0)?,proof:r.get(1)?})).optional().map_err(|_|AppError::internal())
    }
    pub(crate) fn received_image_link_store(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>> {
        self.connection()?.query_row("SELECT CASE WHEN length(r.asset_id)<=64 THEN r.asset_id END,CASE WHEN length(r.proof)=64 THEN r.proof END FROM received_image_assets r JOIN file_assets f ON f.id=r.asset_id AND f.account_id=r.account_id WHERE r.account_id=?1 AND r.observation_id=?2 AND f.expires_at>?3",params![account,observation_id,Utc::now().to_rfc3339()],|r|Ok(werc_domain::received_media::ReceivedImageLink {asset_id:r.get(0)?,proof:r.get(1)?})).optional().map_err(|_|AppError::internal())
    }
    pub(crate) fn received_file_link_store(
        &self,
        account: &str,
        observation_id: &str,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedImageLink>> {
        self.connection()?.query_row("SELECT CASE WHEN length(r.asset_id)<=64 THEN r.asset_id END,CASE WHEN length(r.proof)=64 THEN r.proof END FROM received_file_assets r JOIN file_assets f ON f.id=r.asset_id AND f.account_id=r.account_id WHERE r.account_id=?1 AND r.observation_id=?2 AND f.expires_at>?3",params![account,observation_id,Utc::now().to_rfc3339()],|r|Ok(werc_domain::received_media::ReceivedImageLink {asset_id:r.get(0)?,proof:r.get(1)?})).optional().map_err(|_|AppError::internal())
    }
    pub(crate) fn save_file_store_inner(
        &self,
        data: &ProtectedFile,
        binding: Option<&werc_domain::received_media::ReceivedImageBinding<'_>>,
    ) -> AppResult<()> {
        self.save_file_store_kind(data, binding, false)
    }
    pub(crate) fn save_file_store_kind(
        &self,
        data: &ProtectedFile,
        binding: Option<&werc_domain::received_media::ReceivedImageBinding<'_>>,
        ordinary_file: bool,
    ) -> AppResult<()> {
        self.save_file_checked_store(data, binding, ordinary_file, None)
    }
    pub(crate) fn save_file_checked_store(
        &self,
        data: &ProtectedFile,
        binding: Option<&werc_domain::received_media::ReceivedImageBinding<'_>>,
        ordinary_file: bool,
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
                    "连接或密钥已变化，媒体未保存",
                ));
            }
        }
        let now = Utc::now().to_rfc3339();
        if let Some(binding) = binding {
            crate::current_store::require_current(&tx, &data.asset.account_id)?;
            let revision: i64 = tx
                .query_row(
                    "SELECT revision FROM current_connection WHERE singleton=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(|_| AppError::internal())?;
            if revision != binding.connection_revision {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "当前连接配置已变化，未保存媒体",
                ));
            }
            if binding.observation_id.len() != 64
                || binding.proof.len() != 64
                || !binding.proof.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(AppError::new("RECEIVED_MEDIA_INVALID", "媒体关联格式无效"));
            }
        }
        Self::insert_file_asset(&tx, data)?;
        if let Some(binding) = binding {
            tx.execute(if ordinary_file { "INSERT INTO received_file_assets(account_id,observation_id,asset_id,proof) VALUES(?1,?2,?3,?4)" } else { "INSERT INTO received_image_assets(account_id,observation_id,asset_id,proof) VALUES(?1,?2,?3,?4)" },params![data.asset.account_id,binding.observation_id,data.asset.id,binding.proof]).map_err(|_|AppError::new("RECEIVED_MEDIA_CONFLICT","媒体观测已保存或已变化；未覆盖"))?;
        }
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'file.uploaded',?2,?3)", params![data.asset.account_id, now, serde_json::json!({"asset_id":data.asset.id,"size":data.asset.size,"sent":false}).to_string()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn insert_file_asset(
        tx: &rusqlite::Transaction<'_>,
        data: &ProtectedFile,
    ) -> AppResult<()> {
        if data.asset.size == 0
            || data.asset.size > MAX_FILE_BYTES
            || !(data.asset.size + 19..=data.asset.size + 138).contains(&data.ciphertext.len())
            || data.nonce.len() != 12
            || data.wrapped_key.len() > 16384
            || data.wrapped_key.is_empty()
        {
            return Err(AppError::new("ASSET_CORRUPTED", "文件保护格式无效"));
        }
        let now = Utc::now().to_rfc3339();
        tx.execute("DELETE FROM file_assets WHERE expires_at<=?1", [&now])
            .map_err(|_| AppError::internal())?;
        let (size, count): (i64, i64) = tx
            .query_row(
                "SELECT coalesce(sum(size),0),count(*) FROM file_assets WHERE account_id=?1",
                [&data.asset.account_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| AppError::internal())?;
        if size + data.asset.size as i64 > MAX_ACCOUNT_ASSET_BYTES as i64 || count >= 128 {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "账号文件配额为 64 MiB / 128 项，请先清理资源",
            ));
        }
        let metadata = serde_json::to_string(&data.asset).map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO file_assets(id,account_id,metadata,size,wrapped_key,nonce,ciphertext,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", params![data.asset.id, data.asset.account_id, metadata, data.asset.size as i64, data.wrapped_key, data.nonce, data.ciphertext, data.asset.expires_at]).map_err(|_| AppError::internal())?;
        Ok(())
    }
    pub(crate) fn files_store(&self, account: &str) -> AppResult<Vec<FileAsset>> {
        let c = self.connection()?;
        let mut s = c.prepare("SELECT metadata FROM file_assets WHERE account_id=?1 AND expires_at>?2 ORDER BY rowid DESC LIMIT 128").map_err(|_| AppError::internal())?;
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
    pub(crate) fn file_store(&self, account: &str, id: &str) -> AppResult<ProtectedFile> {
        let c = self.connection()?;
        let row = c.query_row("SELECT metadata,wrapped_key,nonce,ciphertext FROM file_assets WHERE account_id=?1 AND id=?2 AND expires_at>?3 AND length(metadata)<=4096 AND length(ciphertext)<=8388746 AND length(wrapped_key)<=16384 AND length(nonce)=12", params![account, id, Utc::now().to_rfc3339()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, Vec<u8>>(3)?))).optional().map_err(|_| AppError::internal())?.ok_or_else(|| AppError::new("ASSET_NOT_FOUND", "文件不存在或已过期"))?;
        Ok(ProtectedFile {
            asset: serde_json::from_str(&row.0).map_err(|_| AppError::internal())?,
            wrapped_key: row.1,
            nonce: row.2,
            ciphertext: row.3,
        })
    }
    pub(crate) fn delete_file_store(&self, account: &str, id: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        if tx
            .execute(
                "DELETE FROM file_assets WHERE account_id=?1 AND id=?2",
                params![account, id],
            )
            .map_err(|_| AppError::internal())?
            == 0
        {
            return Err(AppError::new("ASSET_NOT_FOUND", "文件不存在"));
        }
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES (?1,'file.removed',?2,?3)", params![account, Utc::now().to_rfc3339(), serde_json::json!({"asset_id":id}).to_string()]).map_err(|_| AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub fn purge_expired_files(&self) -> AppResult<usize> {
        self.connection()?
            .execute(
                "DELETE FROM file_assets WHERE expires_at<=?1",
                [Utc::now().to_rfc3339()],
            )
            .map_err(|_| AppError::internal())
    }
}
