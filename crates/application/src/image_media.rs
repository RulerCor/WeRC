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
    image::{ImageAsset, ProtectedImage, canonical_upload_png},
};
use zeroize::Zeroizing;

impl Application {
    pub fn upload_image(
        &self,
        principal: &Principal,
        account: &str,
        bytes: Vec<u8>,
    ) -> AppResult<ImageAsset> {
        principal.require("media:write")?;
        principal.require_account(account)?;
        self.upload_image_local(account, bytes)
    }
    pub fn upload_image_local(&self, account: &str, bytes: Vec<u8>) -> AppResult<ImageAsset> {
        let bytes = Zeroizing::new(bytes);
        self.ensure_account(account)?;
        let source_sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
        let (image, bytes) = canonical_upload_png(&bytes)?;
        let now = Utc::now();
        let asset = ImageAsset {
            id: Uuid::new_v4().to_string(),
            account_id: account.into(),
            sha256: hex::encode(Sha256::digest(bytes.as_slice())),
            size: bytes.len(),
            image,
            source_sha256,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::hours(24)).to_rfc3339(),
        };
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let context = format!("WeRC/image/v1/{account}/{}", asset.id);
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
        self.repository.save_image(&ProtectedImage {
            asset: asset.clone(),
            wrapped_key,
            nonce: nonce.to_vec(),
            ciphertext,
        })?;
        Ok(asset)
    }
    pub fn image_assets(&self, principal: &Principal, account: &str) -> AppResult<Vec<ImageAsset>> {
        principal.require("media:read")?;
        principal.require_account(account)?;
        self.image_assets_local(account)
    }
    pub fn image_assets_local(&self, account: &str) -> AppResult<Vec<ImageAsset>> {
        self.ensure_account(account)?;
        self.repository.images(account)
    }
    pub fn download_image(
        &self,
        principal: &Principal,
        account: &str,
        id: &str,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        principal.require("media:read")?;
        principal.require_account(account)?;
        self.decrypt_image_local(account, id)
    }
    pub fn image_preview_local(&self, account: &str, id: &str) -> AppResult<Zeroizing<Vec<u8>>> {
        self.ensure_account(account)?;
        self.decrypt_image_local(account, id)
    }
    pub(crate) fn decrypt_image_local(
        &self,
        account: &str,
        id: &str,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        let protected = self.repository.image(account, id)?;
        if protected.nonce.len() != 12
            || protected.asset.account_id != account
            || protected.asset.id != id
        {
            return Err(AppError::new("ASSET_CORRUPTED", "图片保护内容无效"));
        }
        let context = format!("WeRC/image/v1/{account}/{id}");
        let key = Zeroizing::new(
            self.provider
                .unprotect_key(&context, &protected.wrapped_key)?,
        );
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| AppError::new("ASSET_CORRUPTED", "图片保护内容无效"))?;
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
                .map_err(|_| AppError::new("ASSET_CORRUPTED", "图片完整性校验失败"))?,
        );
        if bytes.len() != protected.asset.size
            || hex::encode(Sha256::digest(bytes.as_slice())) != protected.asset.sha256
        {
            return Err(AppError::new("ASSET_CORRUPTED", "图片摘要校验失败"));
        }
        Ok(bytes)
    }
    pub fn delete_image(&self, principal: &Principal, account: &str, id: &str) -> AppResult<()> {
        principal.require("media:write")?;
        principal.require_account(account)?;
        self.delete_image_local(account, id)
    }
    pub fn delete_image_local(&self, account: &str, id: &str) -> AppResult<()> {
        self.ensure_account(account)?;
        self.repository.delete_image(account, id)
    }
}
