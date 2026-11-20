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
    file::{FileAsset, FileContents, FilePreview, ProtectedFile, pack_file, unpack_file},
};
use zeroize::Zeroizing;

impl Application {
    pub fn upload_file(
        &self,
        principal: &Principal,
        account: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> AppResult<FileAsset> {
        principal.require("media:write")?;
        principal.require_account(account)?;
        self.upload_file_local(account, file_name, bytes)
    }
    pub fn upload_file_local(
        &self,
        account: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> AppResult<FileAsset> {
        let protected = self.encrypt_file_local(account, file_name, bytes)?;
        self.repository.save_file(&protected)?;
        Ok(protected.asset)
    }
    pub(crate) fn encrypt_file_local(
        &self,
        account: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> AppResult<ProtectedFile> {
        let bytes = Zeroizing::new(bytes);
        self.ensure_account(account)?;
        let sha256 = hex::encode(Sha256::digest(bytes.as_slice()));
        let size = bytes.len();
        let bytes = pack_file(file_name, &bytes)?;
        let now = Utc::now();
        let asset = FileAsset {
            id: Uuid::new_v4().to_string(),
            account_id: account.into(),
            sha256,
            size,
            created_at: now.to_rfc3339(),
            expires_at: (now + Duration::hours(24)).to_rfc3339(),
        };
        let mut key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let context = format!("WeRC/file/v1/{account}/{}", asset.id);
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
        Ok(ProtectedFile {
            asset,
            wrapped_key,
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }
    pub fn file_assets(&self, principal: &Principal, account: &str) -> AppResult<Vec<FileAsset>> {
        principal.require("media:read")?;
        principal.require_account(account)?;
        self.file_assets_local(account)
    }
    pub fn file_assets_local(&self, account: &str) -> AppResult<Vec<FileAsset>> {
        self.ensure_account(account)?;
        self.repository.files(account)
    }
    pub fn download_file(
        &self,
        principal: &Principal,
        account: &str,
        id: &str,
    ) -> AppResult<FileContents> {
        principal.require("media:read")?;
        principal.require_account(account)?;
        self.decrypt_file_local(account, id)
    }
    pub fn file_preview_local(&self, account: &str, id: &str) -> AppResult<FilePreview> {
        self.ensure_account(account)?;
        let contents = self.decrypt_file_local(account, id)?;
        Ok(FilePreview {
            asset: self.repository.file(account, id)?.asset,
            file_name: contents.file_name.clone(),
        })
    }
    pub(crate) fn decrypt_file_local(&self, account: &str, id: &str) -> AppResult<FileContents> {
        let protected = self.repository.file(account, id)?;
        if protected.nonce.len() != 12
            || protected.asset.account_id != account
            || protected.asset.id != id
        {
            return Err(AppError::new("ASSET_CORRUPTED", "文件保护内容无效"));
        }
        let context = format!("WeRC/file/v1/{account}/{id}");
        let key = Zeroizing::new(
            self.provider
                .unprotect_key(&context, &protected.wrapped_key)?,
        );
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| AppError::new("ASSET_CORRUPTED", "文件保护内容无效"))?;
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
                .map_err(|_| AppError::new("ASSET_CORRUPTED", "文件完整性校验失败"))?,
        );
        let contents = unpack_file(&bytes)?;
        if contents.bytes.len() != protected.asset.size
            || hex::encode(Sha256::digest(contents.bytes.as_slice())) != protected.asset.sha256
        {
            return Err(AppError::new("ASSET_CORRUPTED", "文件摘要校验失败"));
        }
        Ok(contents)
    }
    pub fn delete_file(&self, principal: &Principal, account: &str, id: &str) -> AppResult<()> {
        principal.require("media:write")?;
        principal.require_account(account)?;
        self.delete_file_local(account, id)
    }
    pub fn delete_file_local(&self, account: &str, id: &str) -> AppResult<()> {
        self.ensure_account(account)?;
        self.repository.delete_file(account, id)
    }
}
