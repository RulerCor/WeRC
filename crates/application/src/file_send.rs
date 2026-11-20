use crate::Application;
use werc_domain::{AppError, AppResult, Principal, file::FileContents, outbox::*};
impl Application {
    pub fn create_file_send_local(
        &self,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_file_send("desktop", account, target, asset_id, key)
    }
    pub fn create_file_send(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("messages:send_file")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            target,
            key,
            crate::outbox::ClientContent::Asset(4, asset_id),
        )? {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, target)?;
        let draft = self.prepare_file_send(&p.id, account, target, asset_id, key)?;
        self.finish_client_send(p, draft)
    }
    fn prepare_file_send(
        &self,
        creator: &str,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        if !(16..=128).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "幂等键需16–128个可见ASCII字符",
            ));
        }
        let (context, revision) = self.checked_send_context(account, target)?;
        let preview = self.file_preview_local(account, asset_id)?;
        let file = FileSendAsset {
            asset_id: asset_id.into(),
            sha256: preview.asset.sha256,
            file_name: preview.file_name,
            size: preview.asset.size,
        };
        self.persist_send(
            creator,
            account,
            key,
            revision,
            TextSendPayload {
                moment: None,
                mentions: Vec::new(),
                quote: None,
                context,
                text: String::new(),
                voice: None,
                image: None,
                file: Some(file),
                video: None,
                emoticon: None,
            },
        )
    }
    pub(crate) fn prepared_file(
        &self,
        account: &str,
        file: &FileSendAsset,
    ) -> AppResult<FileContents> {
        use sha2::{Digest, Sha256};
        let contents = self.decrypt_file_local(account, &file.asset_id)?;
        if contents.file_name != file.file_name
            || contents.bytes.len() != file.size
            || hex::encode(Sha256::digest(&contents.bytes)) != file.sha256
        {
            return Err(AppError::new(
                "ASSET_CORRUPTED",
                "文件名称、大小或摘要与批准内容不符；未提交",
            ));
        }
        Ok(contents)
    }
}
