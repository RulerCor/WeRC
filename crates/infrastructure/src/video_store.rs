use crate::SqliteRepository;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use werc_domain::{
    AppError, AppResult,
    video::{MAX_VIDEO_BYTES, ProtectedVideo, VideoAsset, VideoBinding},
};

impl SqliteRepository {
    pub(crate) fn save_video_store(
        &self,
        data: &ProtectedVideo,
        binding: &VideoBinding<'_>,
    ) -> AppResult<()> {
        if data.asset.size == 0
            || data.asset.size > MAX_VIDEO_BYTES
            || data.ciphertext.len() != data.asset.size + 16
            || data.nonce.len() != 12
            || data.wrapped_key.is_empty()
            || data.wrapped_key.len() > 16384
            || binding.observation_id.len() != 64
            || binding.proof.len() != 64
            || !binding.proof.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(AppError::new("ASSET_CORRUPTED", "视频保护格式无效"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AppError::internal())?;
        crate::current_store::require_current(&tx, &data.asset.account_id)?;
        let revision: i64 = tx
            .query_row(
                "SELECT revision FROM current_connection WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| AppError::internal())?;
        let key_revision:i64=tx.query_row("SELECT COALESCE(MAX(seq),0) FROM events WHERE account_id=?1 AND event_type IN ('key.protected','key.removed','connection.bound','connection.unbound')",[&data.asset.account_id],|r|r.get(0)).map_err(|_|AppError::internal())?;
        if (revision, key_revision) != (binding.connection_revision, binding.key_revision) {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "连接或密钥已变化，视频未保存",
            ));
        }
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "DELETE FROM received_video_assets WHERE expires_at<=?1",
            [&now],
        )
        .map_err(|_| AppError::internal())?;
        let (size,count):(i64,i64)=tx.query_row("SELECT coalesce(sum(size),0),count(*) FROM received_video_assets WHERE account_id=?1",[&data.asset.account_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|AppError::internal())?;
        if size + data.asset.size as i64 > 64 * 1024 * 1024 || count >= 32 {
            return Err(AppError::new(
                "QUOTA_EXCEEDED",
                "视频缓存最多64MiB / 32项，请先清理已保存视频",
            ));
        }
        let metadata = serde_json::to_string(&data.asset).map_err(|_| AppError::internal())?;
        tx.execute("INSERT INTO received_video_assets(id,account_id,observation_id,proof,metadata,size,wrapped_key,nonce,ciphertext,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![data.asset.id,data.asset.account_id,binding.observation_id,binding.proof,metadata,data.asset.size as i64,data.wrapped_key,data.nonce,data.ciphertext,data.asset.expires_at]).map_err(|_|AppError::new("RECEIVED_MEDIA_CONFLICT","视频已保存或观测已变化，未覆盖"))?;
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'video.received',?2,?3)",params![data.asset.account_id,now,serde_json::json!({"asset_id":data.asset.id,"size":data.asset.size,"sent":false}).to_string()]).map_err(|_|AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub(crate) fn video_info_store(
        &self,
        account: &str,
        observation: &str,
    ) -> AppResult<Option<(VideoAsset, String)>> {
        let c = self.connection()?;
        let row:Option<(String,String)>=c.query_row("SELECT CASE WHEN length(metadata)<=4096 THEN metadata END,CASE WHEN length(proof)=64 THEN proof END FROM received_video_assets WHERE account_id=?1 AND observation_id=?2 AND expires_at>?3",params![account,observation,Utc::now().to_rfc3339()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|AppError::internal())?;
        row.map(|(m, p)| {
            Ok((
                serde_json::from_str(&m)
                    .map_err(|_| AppError::new("ASSET_CORRUPTED", "视频信息无效"))?,
                p,
            ))
        })
        .transpose()
    }
    pub(crate) fn video_store(
        &self,
        account: &str,
        observation: &str,
    ) -> AppResult<ProtectedVideo> {
        let c = self.connection()?;
        let row=c.query_row("SELECT metadata,wrapped_key,nonce,ciphertext FROM received_video_assets WHERE account_id=?1 AND observation_id=?2 AND expires_at>?3 AND length(metadata)<=4096 AND length(wrapped_key) BETWEEN 1 AND 16384 AND length(nonce)=12 AND length(ciphertext)=size+16 AND size BETWEEN 1 AND 16777216",params![account,observation,Utc::now().to_rfc3339()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,Vec<u8>>(3)?))).optional().map_err(|_|AppError::internal())?.ok_or_else(||AppError::new("ASSET_NOT_FOUND","视频未保存、已过期或保护内容无效"))?;
        Ok(ProtectedVideo {
            asset: serde_json::from_str(&row.0).map_err(|_| AppError::internal())?,
            wrapped_key: row.1,
            nonce: row.2,
            ciphertext: row.3,
        })
    }
    pub(crate) fn delete_video_store(&self, account: &str, observation: &str) -> AppResult<()> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| AppError::internal())?;
        let changed = tx
            .execute(
                "DELETE FROM received_video_assets WHERE account_id=?1 AND observation_id=?2",
                params![account, observation],
            )
            .map_err(|_| AppError::internal())?;
        if changed == 0 {
            return Err(AppError::new("ASSET_NOT_FOUND", "视频缓存不存在"));
        }
        tx.execute("INSERT INTO events(account_id,event_type,occurred_at,payload) VALUES(?1,'video.removed',?2,'{}')",params![account,Utc::now().to_rfc3339()]).map_err(|_|AppError::internal())?;
        tx.commit().map_err(|_| AppError::internal())
    }
    pub fn purge_expired_videos(&self) -> AppResult<usize> {
        self.connection()?
            .execute(
                "DELETE FROM received_video_assets WHERE expires_at<=?1",
                [Utc::now().to_rfc3339()],
            )
            .map_err(|_| AppError::internal())
    }
}
