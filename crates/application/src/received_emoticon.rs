use crate::Application;
use werc_domain::{AppError, AppResult, file::FileAsset};

impl Application {
    pub fn received_emoticon_asset_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<Option<FileAsset>> {
        if position < 1 {
            return Err(AppError::new("INVALID_ARGUMENT", "观测位置无效"));
        }
        let row = self
            .received_texts_local(account, position - 1, 1)?
            .into_iter()
            .next()
            .filter(|r| r.position == position)
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "表情观测不存在"))?;
        if !matches!(row.payload.message.message_type, 47 | 49) {
            return Err(AppError::new("RECEIVED_MEDIA_INVALID", "此观测不是表情"));
        }
        let Some(link) = self
            .repository
            .received_file_link(account, &row.payload.id)?
        else {
            return Ok(None);
        };
        let asset = self.repository.file(account, &link.asset_id)?.asset;
        if asset.account_id != account
            || asset.id != link.asset_id
            || link.proof
                != self.send_digest(
                    account,
                    "received-emoticon-link",
                    &(&row.payload.id, &asset.id, &asset.sha256),
                )?
        {
            return Err(AppError::new(
                "RECEIVED_MEDIA_CORRUPTED",
                "表情关联认证失败",
            ));
        }
        Ok(Some(asset))
    }
    pub fn receive_emoticon_local(
        &self,
        account: &str,
        position: i64,
        request_id: &str,
    ) -> AppResult<FileAsset> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一个媒体下载仍在进行"))?;
        if position < 1 {
            return Err(AppError::new("INVALID_ARGUMENT", "观测位置无效"));
        }
        if let Some(asset) = self.received_emoticon_asset_local(account, position)? {
            return Ok(asset);
        }
        let lease = self.browse_lease(request_id)?;
        lease.check()?;
        let key_revision = self.repository.key_revision(account)?;
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let row = self
            .received_texts_local(account, position - 1, 1)?
            .into_iter()
            .next()
            .filter(|r| r.position == position)
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "表情观测不存在"))?;
        if row.payload.sender_id != context.sender_id {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "观测所属登录身份与当前连接不同",
            ));
        }
        let descriptor = werc_domain::received_emoticon::descriptor(&row.payload.message)?;
        let bytes =
            self.provider
                .read_received_emoticon(account, &context, &descriptor, &lease.cancel)?;
        lease.check()?;
        descriptor.validate_bytes(&bytes)?;
        if bytes.starts_with(b"wxgf") {
            self.provider
                .render_received_emoticon(&bytes, &lease.cancel)?;
        }
        if self.checked_send_context(account, "filehelper")? != (context.clone(), revision) {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "表情读取期间连接发生变化",
            ));
        }
        let protected =
            self.encrypt_file_local(account, descriptor.stored_name(&bytes), bytes.to_vec())?;
        let proof = self.send_digest(
            account,
            "received-emoticon-link",
            &(
                &row.payload.id,
                &protected.asset.id,
                &protected.asset.sha256,
            ),
        )?;
        lease.check()?;
        self.repository.save_received_emoticon(
            &protected,
            &werc_domain::received_media::ReceivedImageBinding {
                observation_id: &row.payload.id,
                connection_revision: revision,
                proof: &proof,
            },
            key_revision,
        )?;
        Ok(protected.asset)
    }
    pub fn received_emoticon_content_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<werc_domain::file::FileContents> {
        let asset = self
            .received_emoticon_asset_local(account, position)?
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "尚未下载表情或缓存已过期"))?;
        self.emoticon_contents_local(account, &asset.id)
    }
    pub fn delete_received_emoticon_local(&self, account: &str, position: i64) -> AppResult<()> {
        let asset = self
            .received_emoticon_asset_local(account, position)?
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "表情尚未保存或已过期"))?;
        self.delete_file_local(account, &asset.id)
    }
}
