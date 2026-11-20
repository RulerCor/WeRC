use crate::Application;
use werc_domain::{AppError, AppResult, Principal, file::FileContents, outbox::*};
impl Application {
    pub fn create_video_send_local(
        &self,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_video_send("desktop", account, target, asset_id, key)
    }
    pub fn create_video_send(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("messages:send_video")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            target,
            key,
            crate::outbox::ClientContent::Asset(5, asset_id),
        )? {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, target)?;
        let draft = self.prepare_video_send(&p.id, account, target, asset_id, key)?;
        self.finish_client_send(p, draft)
    }
    fn prepare_video_send(
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
        let info = werc_domain::video::inspect_send_mp4(&contents.file_name, &contents.bytes)?;
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
                video: Some(VideoSendAsset { file, info }),
                emoticon: None,
            },
        )
    }
    pub(crate) fn prepared_video(
        &self,
        account: &str,
        video: &VideoSendAsset,
    ) -> AppResult<FileContents> {
        let contents = self.prepared_file(account, &video.file)?;
        if werc_domain::video::inspect_send_mp4(&contents.file_name, &contents.bytes)? != video.info
        {
            return Err(AppError::new(
                "ASSET_CORRUPTED",
                "视频参数与批准内容不符；未提交",
            ));
        }
        Ok(contents)
    }
    pub fn upload_video_local(
        &self,
        account: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> AppResult<werc_domain::file::FileAsset> {
        werc_domain::video::inspect_send_mp4(name, &bytes)?;
        self.upload_file_local(account, name, bytes)
    }
    pub fn video_preview_local(&self, account: &str, id: &str) -> AppResult<VideoSendAsset> {
        let preview = self.file_preview_local(account, id)?;
        let contents = self.decrypt_file_local(account, id)?;
        let info = werc_domain::video::inspect_send_mp4(&contents.file_name, &contents.bytes)?;
        Ok(VideoSendAsset {
            file: FileSendAsset {
                asset_id: id.into(),
                sha256: preview.asset.sha256,
                file_name: preview.file_name,
                size: preview.asset.size,
            },
            info,
        })
    }
    pub fn video_contents_local(&self, account: &str, id: &str) -> AppResult<FileContents> {
        self.ensure_account(account)?;
        let contents = self.decrypt_file_local(account, id)?;
        werc_domain::video::inspect_send_mp4(&contents.file_name, &contents.bytes)?;
        Ok(contents)
    }
}
