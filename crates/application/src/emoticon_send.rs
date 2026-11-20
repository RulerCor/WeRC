use crate::Application;
use werc_domain::{AppError, AppResult, Principal, file::FileContents, outbox::*};
impl Application {
    pub fn create_emoticon_send_local(
        &self,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_emoticon_send("desktop", account, target, asset_id, key)
    }
    pub fn create_emoticon_send(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("messages:send_emoticon")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            target,
            key,
            crate::outbox::ClientContent::Asset(6, asset_id),
        )? {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, target)?;
        let draft = self.prepare_emoticon_send(&p.id, account, target, asset_id, key)?;
        self.finish_client_send(p, draft)
    }
    fn prepare_emoticon_send(
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
        let contents = self.decrypt_file_local(account, asset_id)?;
        let info = werc_domain::emoticon::inspect_gif(&contents.file_name, &contents.bytes)?;
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
                file: None,
                emoticon: Some(EmoticonSendAsset { file, info }),
                video: None,
            },
        )
    }
    pub(crate) fn prepared_emoticon(
        &self,
        account: &str,
        emoticon: &EmoticonSendAsset,
    ) -> AppResult<FileContents> {
        let contents = self.prepared_file(account, &emoticon.file)?;
        if werc_domain::emoticon::inspect_gif(&contents.file_name, &contents.bytes)?
            != emoticon.info
        {
            return Err(AppError::new(
                "ASSET_CORRUPTED",
                "表情参数与批准内容不符；未提交",
            ));
        }
        Ok(contents)
    }
    pub fn upload_emoticon_local(
        &self,
        account: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> AppResult<werc_domain::file::FileAsset> {
        werc_domain::emoticon::inspect_gif(name, &bytes)?;
        self.upload_file_local(account, name, bytes)
    }
    pub fn emoticon_preview_local(&self, account: &str, id: &str) -> AppResult<EmoticonSendAsset> {
        let preview = self.file_preview_local(account, id)?;
        let contents = self.decrypt_file_local(account, id)?;
        let info = werc_domain::emoticon::inspect_gif(&contents.file_name, &contents.bytes)?;
        Ok(EmoticonSendAsset {
            file: FileSendAsset {
                asset_id: id.into(),
                sha256: preview.asset.sha256,
                file_name: preview.file_name,
                size: preview.asset.size,
            },
            info,
        })
    }
    pub fn emoticon_contents_local(&self, account: &str, id: &str) -> AppResult<FileContents> {
        self.ensure_account(account)?;
        let contents = self.decrypt_file_local(account, id)?;
        if contents.file_name == "received-emoticon.wxgf" {
            werc_domain::wxgf::parse(&contents.bytes)?;
            let bytes = self.provider.render_received_emoticon(
                &contents.bytes,
                &std::sync::atomic::AtomicBool::new(false),
            )?;
            return Ok(FileContents {
                file_name: "received-emoticon.png".into(),
                bytes,
            });
        }
        werc_domain::emoticon::inspect_gif(&contents.file_name, &contents.bytes)?;
        Ok(contents)
    }
}
