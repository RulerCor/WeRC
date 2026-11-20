use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use werc_domain::{
    AppError, AppResult,
    receive::ReceivedTextRow,
    video::{ProtectedVideo, VideoAsset, VideoBinding, inspect_mp4},
};
use zeroize::Zeroizing;

impl Application {
    pub(crate) fn encrypt_video_local(
        &self,
        account: &str,
        bytes: &[u8],
    ) -> AppResult<ProtectedVideo> {
        self.ensure_account(account)?;
        let video = inspect_mp4(bytes)?;
        let now = Utc::now();
        let asset = VideoAsset {
            id: Uuid::new_v4().to_string(),
            account_id: account.into(),
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len(),
            video,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::hours(24)).to_rfc3339(),
        };
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let wrapping_context = format!("WeRC/video/v1/{account}/{}", asset.id);
        let wrapped_key = self.provider.protect_key(&wrapping_context, key.as_ref())?;
        let metadata = serde_json::to_vec(&asset).map_err(|_| AppError::internal())?;
        let ciphertext = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|_| AppError::internal())?
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: &metadata,
                },
            )
            .map_err(|_| AppError::internal())?;
        Ok(ProtectedVideo {
            asset,
            wrapped_key,
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    fn video_row(&self, account: &str, position: i64) -> AppResult<ReceivedTextRow> {
        if position < 1 {
            return Err(AppError::new("INVALID_ARGUMENT", "观测位置无效"));
        }
        let row = self
            .received_texts_local(account, position - 1, 1)?
            .into_iter()
            .next()
            .filter(|r| r.position == position)
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "视频观测不存在"))?;
        if row.payload.message.message_type != 43 {
            return Err(AppError::new(
                "RECEIVED_MEDIA_INVALID",
                "此观测不是已支持的视频",
            ));
        }
        Ok(row)
    }
    pub fn received_video_asset_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<Option<VideoAsset>> {
        let row = self.video_row(account, position)?;
        let Some((asset, proof)) = self
            .repository
            .received_video_info(account, &row.payload.id)?
        else {
            return Ok(None);
        };
        if asset.account_id != account
            || proof
                != self.send_digest(account, "received-video-link", &(&row.payload.id, &asset))?
        {
            return Err(AppError::new(
                "RECEIVED_MEDIA_CORRUPTED",
                "视频关联认证失败",
            ));
        }
        Ok(Some(asset))
    }
    pub fn receive_video_local(&self, account: &str, position: i64) -> AppResult<VideoAsset> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一个媒体下载仍在进行"))?;
        if let Some(asset) = self.received_video_asset_local(account, position)? {
            return Ok(asset);
        }
        let row = self.video_row(account, position)?;
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let key_revision = self.repository.key_revision(account)?;
        if row.payload.sender_id != context.sender_id {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "观测所属身份与当前连接不同",
            ));
        }
        let descriptor = werc_domain::received_media::video_descriptor(&row.payload.message)?;
        let bytes = self
            .provider
            .download_received_video(&context, &descriptor)?;
        let protected = self.encrypt_video_local(account, &bytes)?;
        let proof = self.send_digest(
            account,
            "received-video-link",
            &(&row.payload.id, &protected.asset),
        )?;
        if self.checked_send_context(account, "filehelper")? != (context, revision)
            || self.repository.key_revision(account)? != key_revision
        {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "视频处理期间连接或密钥已变化",
            ));
        }
        self.repository.save_received_video(
            &protected,
            &VideoBinding {
                observation_id: &row.payload.id,
                connection_revision: revision,
                key_revision,
                proof: &proof,
            },
        )?;
        Ok(protected.asset)
    }
    pub fn received_video_content_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        let asset = self
            .received_video_asset_local(account, position)?
            .ok_or_else(|| AppError::new("ASSET_NOT_FOUND", "视频未保存或已过期"))?;
        let row = self.video_row(account, position)?;
        let protected = self.repository.received_video(account, &row.payload.id)?;
        self.decrypt_video_local(account, &asset, protected)
    }
    pub(crate) fn decrypt_video_local(
        &self,
        account: &str,
        asset: &VideoAsset,
        protected: ProtectedVideo,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        let metadata = serde_json::to_vec(&protected.asset).map_err(|_| AppError::internal())?;
        if metadata != serde_json::to_vec(&asset).map_err(|_| AppError::internal())?
            || protected.nonce.len() != 12
        {
            return Err(AppError::new("ASSET_CORRUPTED", "视频缓存已变化"));
        }
        let key = Zeroizing::new(self.provider.unprotect_key(
            &format!("WeRC/video/v1/{account}/{}", asset.id),
            &protected.wrapped_key,
        )?);
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| AppError::new("ASSET_CORRUPTED", "视频保护密钥无效"))?;
        let bytes = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&protected.nonce),
                    Payload {
                        msg: &protected.ciphertext,
                        aad: &metadata,
                    },
                )
                .map_err(|_| AppError::new("ASSET_CORRUPTED", "视频完整性校验失败"))?,
        );
        if bytes.len() != asset.size
            || hex::encode(Sha256::digest(&bytes)) != asset.sha256
            || inspect_mp4(&bytes)? != asset.video
        {
            return Err(AppError::new("ASSET_CORRUPTED", "视频内容与信息不符"));
        }
        Ok(bytes)
    }
    pub fn delete_received_video_local(&self, account: &str, position: i64) -> AppResult<()> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一个媒体下载仍在进行"))?;
        let row = self.video_row(account, position)?;
        self.repository
            .delete_received_video(account, &row.payload.id)
    }
}
