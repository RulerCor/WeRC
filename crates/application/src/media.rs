use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use werc_domain::{
    AppError, AppResult, Principal,
    media::{AudioAsset, ProtectedAsset, inspect_wav},
};
use zeroize::Zeroizing;

impl Application {
    pub fn upload_audio(
        &self,
        principal: &Principal,
        account: &str,
        bytes: Vec<u8>,
    ) -> AppResult<AudioAsset> {
        principal.require("media:write")?;
        principal.require_account(account)?;
        self.upload_audio_local(account, bytes)
    }
    pub fn upload_audio_local(&self, account: &str, bytes: Vec<u8>) -> AppResult<AudioAsset> {
        let protected = self.encrypt_audio_local(account, bytes)?;
        self.repository.save_asset(&protected)?;
        Ok(protected.asset)
    }
    pub(crate) fn encrypt_audio_local(
        &self,
        account: &str,
        bytes: Vec<u8>,
    ) -> AppResult<ProtectedAsset> {
        let bytes = Zeroizing::new(bytes);
        self.ensure_account(account)?;
        let audio = inspect_wav(&bytes)?;
        let now = Utc::now();
        let asset = AudioAsset {
            id: Uuid::new_v4().to_string(),
            account_id: account.into(),
            sha256: hex::encode(Sha256::digest(bytes.as_slice())),
            size: bytes.len(),
            audio,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::hours(24)).to_rfc3339(),
        };
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let context = format!("WeRC/audio/v1/{account}/{}", asset.id);
        let wrapped_key = self.provider.protect_key(&context, key.as_ref())?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| AppError::internal())?;
        let metadata = serde_json::to_vec(&asset).map_err(|_| AppError::internal())?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes.as_slice(),
                    aad: &metadata,
                },
            )
            .map_err(|_| AppError::internal())?;
        Ok(ProtectedAsset {
            asset,
            wrapped_key,
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }
    pub fn audio_assets(&self, principal: &Principal, account: &str) -> AppResult<Vec<AudioAsset>> {
        principal.require("media:read")?;
        principal.require_account(account)?;
        self.audio_assets_local(account)
    }
    pub fn audio_assets_local(&self, account: &str) -> AppResult<Vec<AudioAsset>> {
        self.ensure_account(account)?;
        self.repository.assets(account)
    }
    pub fn download_audio(
        &self,
        principal: &Principal,
        account: &str,
        id: &str,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        principal.require("media:read")?;
        principal.require_account(account)?;
        self.decrypt_audio_local(account, id)
    }
    pub fn audio_preview_local(&self, account: &str, id: &str) -> AppResult<Zeroizing<Vec<u8>>> {
        self.ensure_account(account)?;
        self.decrypt_audio_local(account, id)
    }
    pub(crate) fn decrypt_audio_local(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        let protected = self.repository.asset(account, id)?;
        if protected.nonce.len() != 12 {
            return Err(AppError::new("ASSET_CORRUPTED", "音频保护内容无效"));
        }
        let context = format!("WeRC/audio/v1/{account}/{id}");
        let key = Zeroizing::new(
            self.provider
                .unprotect_key(&context, &protected.wrapped_key)?,
        );
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| AppError::new("ASSET_CORRUPTED", "音频保护内容无效"))?;
        let metadata = serde_json::to_vec(&protected.asset).map_err(|_| AppError::internal())?;
        let bytes = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&protected.nonce),
                    Payload {
                        msg: &protected.ciphertext,
                        aad: &metadata,
                    },
                )
                .map_err(|_| AppError::new("ASSET_CORRUPTED", "音频完整性校验失败"))?,
        );
        if bytes.len() != protected.asset.size
            || hex::encode(Sha256::digest(bytes.as_slice())) != protected.asset.sha256
        {
            return Err(AppError::new("ASSET_CORRUPTED", "音频摘要校验失败"));
        }
        Ok(bytes)
    }
    pub fn delete_audio(&self, principal: &Principal, account: &str, id: &str) -> AppResult<()> {
        principal.require("media:write")?;
        principal.require_account(account)?;
        self.delete_audio_local(account, id)
    }
    pub fn delete_audio_local(&self, account: &str, id: &str) -> AppResult<()> {
        self.ensure_account(account)?;
        self.repository.delete_asset(account, id)
    }
}
