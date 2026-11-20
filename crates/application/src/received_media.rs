use crate::Application;
use werc_domain::{AppError, AppResult, file::FileAsset, received_media::image_descriptor};

impl Application {
    fn live_file_descriptor(
        &self,
        account: &str,
        view_id: &str,
        local_id: &str,
    ) -> AppResult<(
        zeroize::Zeroizing<String>,
        String,
        werc_domain::received_media::ReceivedFileDescriptor,
    )> {
        let slot = self.live_result.lock().map_err(|_| AppError::internal())?;
        let result = slot
            .as_ref()
            .filter(|r| r.account == account)
            .ok_or_else(|| AppError::new("CURSOR_STALE", "请重新读取在线窗口"))?;
        let binding = self.live_read_binding(account, &result.database)?.0;
        if binding != result.binding {
            return Err(AppError::new("CURSOR_STALE", "来源或身份已变化"));
        }
        let window = result.view.recovery_window(&binding, view_id)?;
        let row = window
            .rows
            .iter()
            .find(|r| r.local_id == local_id)
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "此窗口中没有指定附件"))?;
        if row
            .message_type
            .parse::<i64>()
            .map_or(true, |t| t & 0xffffffff != 49)
        {
            return Err(AppError::new(
                "RECEIVED_MEDIA_INVALID",
                "此记录不是普通附件",
            ));
        }
        let message = werc_domain::receive::ReceivedText {
            sequence: 1,
            native_message_id: row.server_id.clone().unwrap_or_else(|| "0".into()),
            message_type: 49,
            text: String::new(),
            metadata: row.metadata_body().map(str::to_owned),
        };
        let descriptor = werc_domain::received_media::file_descriptor(&message)?;
        Ok((result.database.clone(), binding, descriptor))
    }
    pub fn live_file_info_local(
        &self,
        account: &str,
        view_id: &str,
        local_id: &str,
    ) -> AppResult<werc_domain::received_media::ReceivedFileInfo> {
        let (_, _, d) = self.live_file_descriptor(account, view_id, local_id)?;
        Ok(werc_domain::received_media::ReceivedFileInfo {
            file_name: d.file_name.to_string(),
            size: d.media.declared_size,
        })
    }
    pub fn download_live_file_local(
        &self,
        account: &str,
        view_id: &str,
        local_id: &str,
    ) -> AppResult<werc_domain::file::FileContents> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一个媒体下载仍在进行"))?;
        let (database, binding, descriptor) =
            self.live_file_descriptor(account, view_id, local_id)?;
        let context = self.checked_send_context(account, "filehelper")?;
        let bytes = self
            .provider
            .download_received_file(&context.0, &descriptor.media)?;
        if bytes.len() as u64 != descriptor.media.declared_size {
            return Err(AppError::new(
                "RECEIVED_MEDIA_INVALID",
                "附件完成字节数与声明不符",
            ));
        }
        if self.checked_send_context(account, "filehelper")? != context
            || self.live_read_binding(account, &database)?.0 != binding
        {
            return Err(AppError::new("CURSOR_STALE", "下载期间来源或身份已变化"));
        }
        // Recheck expiry and view replacement before releasing plaintext to the exporter.
        self.live_file_descriptor(account, view_id, local_id)?;
        Ok(werc_domain::file::FileContents {
            file_name: descriptor.file_name.to_string(),
            bytes,
        })
    }

    pub fn received_file_info_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<Option<werc_domain::received_media::ReceivedFileInfo>> {
        if position < 1 {
            return Err(AppError::new("INVALID_ARGUMENT", "观测位置无效"));
        }
        let row = self
            .received_texts_local(account, position - 1, 1)?
            .into_iter()
            .next()
            .filter(|r| r.position == position)
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "附件观测不存在"))?;
        Ok(
            werc_domain::received_media::file_descriptor(&row.payload.message)
                .ok()
                .map(|d| werc_domain::received_media::ReceivedFileInfo {
                    file_name: d.file_name.to_string(),
                    size: d.media.declared_size,
                }),
        )
    }

    pub fn received_voice_file_asset_local(
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
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "语音观测不存在"))?;
        if row.payload.message.message_type != 34 {
            return Err(AppError::new("RECEIVED_MEDIA_INVALID", "此观测不是语音"));
        }
        let Some(link) = self
            .repository
            .received_voice_link(account, &row.payload.id)?
        else {
            return Ok(None);
        };
        let asset = self.repository.file(account, &link.asset_id)?.asset;
        if asset.account_id != account
            || asset.id != link.asset_id
            || link.proof
                != self.send_digest(
                    account,
                    "received-voice-link",
                    &(&row.payload.id, &asset.id, &asset.sha256),
                )?
        {
            return Err(AppError::new(
                "RECEIVED_MEDIA_CORRUPTED",
                "语音关联认证失败",
            ));
        }
        Ok(Some(asset))
    }
    pub fn received_voice_silk_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        let asset = self
            .received_voice_file_asset_local(account, position)?
            .ok_or_else(|| {
                AppError::new("RECEIVED_MEDIA_NOT_FOUND", "没有保存内联语音或缓存已过期")
            })?;
        let mut file = self.decrypt_file_local(account, &asset.id)?;
        if file.file_name != "received-voice.silk" || file.bytes.len() > 262144 {
            return Err(AppError::new(
                "RECEIVED_MEDIA_CORRUPTED",
                "语音格式关联无效",
            ));
        }
        Ok(std::mem::take(&mut file.bytes))
    }
    pub fn received_voice_preview_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        let silk = self.received_voice_silk_local(account, position)?;
        self.provider.decode_received_voice(&silk)
    }

    pub fn received_image_file_asset_local(
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
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "图片观测不存在"))?;
        if row.payload.message.message_type != 3 {
            return Err(AppError::new("RECEIVED_MEDIA_INVALID", "此观测不是图片"));
        }
        let Some(link) = self
            .repository
            .received_image_link(account, &row.payload.id)?
        else {
            return Ok(None);
        };
        let asset = self.repository.file(account, &link.asset_id)?.asset;
        if asset.account_id != account
            || asset.id != link.asset_id
            || link.proof
                != self.send_digest(
                    account,
                    "received-image-link",
                    &(&row.payload.id, &asset.id, &asset.sha256),
                )?
        {
            return Err(AppError::new(
                "RECEIVED_MEDIA_CORRUPTED",
                "图片关联认证失败",
            ));
        }
        Ok(Some(asset))
    }
    pub fn received_image_preview_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        let asset = self
            .received_image_file_asset_local(account, position)?
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "尚未下载图片或已过期"))?;
        let mut file = self.decrypt_file_local(account, &asset.id)?;
        if file.file_name != "received-image.png" {
            return Err(AppError::new(
                "RECEIVED_MEDIA_CORRUPTED",
                "图片格式关联无效",
            ));
        }
        Ok(std::mem::take(&mut file.bytes))
    }
    /// Download an authenticated stored image observation from the currently
    /// bound native identity. No raw XML/credentials/path supplied by the caller.
    pub fn receive_image_file_local(&self, account: &str, position: i64) -> AppResult<FileAsset> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一个媒体下载仍在进行"))?;
        if position < 1 {
            return Err(AppError::new("INVALID_ARGUMENT", "观测位置无效"));
        }
        if let Some(asset) = self.received_image_file_asset_local(account, position)? {
            return Ok(asset);
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let row = self
            .received_texts_local(account, position - 1, 1)?
            .into_iter()
            .next()
            .filter(|r| r.position == position)
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "图片观测不存在"))?;
        if row.payload.sender_id != context.sender_id {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "观测所属登录身份与当前连接不同",
            ));
        }
        let descriptor = image_descriptor(&row.payload.message)?;
        let bytes = self
            .provider
            .download_received_image(&context, &descriptor)?;
        if bytes.is_empty()
            || bytes.len() as u64 > werc_domain::received_media::RECEIVED_MEDIA_MAX_BYTES
            || self.checked_send_context(account, "filehelper")? != (context.clone(), revision)
        {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "图片下载期间连接或大小发生变化",
            ));
        }
        let (_, canonical) = werc_domain::image::canonical_received_png(&bytes)?;
        if self.checked_send_context(account, "filehelper")? != (context, revision) {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "图片解码期间连接发生变化",
            ));
        }
        // Store only a fully decoded, metadata-free static PNG. No send draft
        // or approval is created by receiving media.
        let protected =
            self.encrypt_file_local(account, "received-image.png", canonical.to_vec())?;
        let proof = self.send_digest(
            account,
            "received-image-link",
            &(
                &row.payload.id,
                &protected.asset.id,
                &protected.asset.sha256,
            ),
        )?;
        self.repository.save_received_image_file(
            &protected,
            &werc_domain::received_media::ReceivedImageBinding {
                observation_id: &row.payload.id,
                connection_revision: revision,
                proof: &proof,
            },
        )?;
        Ok(protected.asset)
    }
    pub fn received_file_asset_local(
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
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "附件观测不存在"))?;
        if row.payload.message.message_type != 49 {
            return Err(AppError::new("RECEIVED_MEDIA_INVALID", "此观测不是附件"));
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
                    "received-file-link",
                    &(&row.payload.id, &asset.id, &asset.sha256),
                )?
        {
            return Err(AppError::new(
                "RECEIVED_MEDIA_CORRUPTED",
                "附件关联认证失败",
            ));
        }
        Ok(Some(asset))
    }
    pub fn receive_file_local(&self, account: &str, position: i64) -> AppResult<FileAsset> {
        let _guard = self
            .received_media_guard
            .try_lock()
            .map_err(|_| AppError::new("RECEIVED_MEDIA_BUSY", "另一个媒体下载仍在进行"))?;
        if position < 1 {
            return Err(AppError::new("INVALID_ARGUMENT", "观测位置无效"));
        }
        if let Some(asset) = self.received_file_asset_local(account, position)? {
            return Ok(asset);
        }
        let (context, revision) = self.checked_send_context(account, "filehelper")?;
        let row = self
            .received_texts_local(account, position - 1, 1)?
            .into_iter()
            .next()
            .filter(|r| r.position == position)
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "附件观测不存在"))?;
        if row.payload.sender_id != context.sender_id {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "观测所属登录身份与当前连接不同",
            ));
        }
        let descriptor = werc_domain::received_media::file_descriptor(&row.payload.message)?;
        let bytes = self
            .provider
            .download_received_file(&context, &descriptor.media)?;
        if bytes.len() as u64 != descriptor.media.declared_size
            || bytes.len() as u64 > werc_domain::received_media::RECEIVED_MEDIA_MAX_BYTES
            || self.checked_send_context(account, "filehelper")? != (context.clone(), revision)
        {
            return Err(AppError::new(
                "RECEIVE_CONTEXT_CHANGED",
                "附件下载期间连接或大小发生变化",
            ));
        }

        // Save exact attachment bytes without executing or previewing them.
        let protected = self.encrypt_file_local(account, &descriptor.file_name, bytes.to_vec())?;
        let proof = self.send_digest(
            account,
            "received-file-link",
            &(
                &row.payload.id,
                &protected.asset.id,
                &protected.asset.sha256,
            ),
        )?;
        self.repository.save_received_file(
            &protected,
            &werc_domain::received_media::ReceivedImageBinding {
                observation_id: &row.payload.id,
                connection_revision: revision,
                proof: &proof,
            },
        )?;
        Ok(protected.asset)
    }
    pub fn received_file_content_local(
        &self,
        account: &str,
        position: i64,
    ) -> AppResult<werc_domain::file::FileContents> {
        let asset = self
            .received_file_asset_local(account, position)?
            .ok_or_else(|| AppError::new("RECEIVED_MEDIA_NOT_FOUND", "尚未下载附件或缓存已过期"))?;
        self.decrypt_file_local(account, &asset.id)
    }
}
