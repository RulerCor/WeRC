use crate::Application;
use werc_domain::{AppError, AppResult, Principal, outbox::*};

impl Application {
    /// The author comes from the native identity, which is resolved
    /// again for approval/submission by the shared encrypted outbox lifecycle.
    pub fn create_private_moment_local(
        &self,
        account: &str,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_moment(
            "desktop",
            account,
            text,
            &[],
            MomentVisibility::Private,
            key,
        )
    }
    pub fn create_private_moment(
        &self,
        p: &Principal,
        account: &str,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("moments:publish")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        self.create_moment(p, account, text, &[], MomentVisibility::Private, key)
    }
    pub fn create_private_image_moment_local(
        &self,
        account: &str,
        text: &str,
        images: &[String],
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_moment(
            "desktop",
            account,
            text,
            images,
            MomentVisibility::Private,
            key,
        )
    }
    pub fn create_private_image_moment(
        &self,
        p: &Principal,
        account: &str,
        text: &str,
        images: &[String],
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("moments:publish")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        self.create_moment(p, account, text, images, MomentVisibility::Private, key)
    }
    pub fn create_moment_local(
        &self,
        account: &str,
        text: &str,
        images: &[String],
        visibility: MomentVisibility,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_moment("desktop", account, text, images, visibility, key)
    }
    pub fn create_moment(
        &self,
        p: &Principal,
        account: &str,
        text: &str,
        images: &[String],
        visibility: MomentVisibility,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("moments:publish")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            "",
            key,
            crate::outbox::ClientContent::Moment(text, images, visibility),
        )? {
            return Ok(existing);
        }
        let draft = self.prepare_moment(&p.id, account, text, images, visibility, key)?;
        self.finish_client_send(p, draft)
    }
    pub fn create_moment_action_local(
        &self,
        account: &str,
        action: &MomentAction,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_moment_action("desktop", account, action, text, key)
    }
    pub fn create_moment_action(
        &self,
        p: &Principal,
        account: &str,
        action: &MomentAction,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("moments:publish")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            "",
            key,
            crate::outbox::ClientContent::MomentAction(text, action),
        )? {
            return Ok(existing);
        }
        let draft = self.prepare_moment_action(&p.id, account, action, text, key)?;
        self.finish_client_send(p, draft)
    }
    fn prepare_moment_action(
        &self,
        creator: &str,
        account: &str,
        action: &MomentAction,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        if !action.valid(text)
            || !(16..=128).contains(&key.len())
            || !key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "请选择精确动态；评论最多1023字节，回复最多895字节并需评论编号和作者；其他操作不附正文",
            ));
        }
        let _guard = self
            .send_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "发送任务正在处理"))?;
        let (identity, _) = self.checked_send_context(account, "filehelper")?;
        let (context, revision) = self.checked_send_context(account, &identity.sender_id)?;
        if context.sender_id != identity.sender_id || context.target_id != context.sender_id {
            return Err(AppError::new("CONNECTION_CHANGED", "当前登录账号已变化"));
        }
        self.persist_send(
            creator,
            account,
            key,
            revision,
            TextSendPayload {
                context,
                text: text.into(),
                moment: Some(MomentPublish {
                    video: None,
                    visibility: MomentVisibility::Private,
                    images: vec![],
                    interaction: Some(action.clone()),
                }),
                mentions: vec![],
                quote: None,
                voice: None,
                image: None,
                file: None,
                video: None,
                emoticon: None,
            },
        )
    }
    pub fn create_video_moment_local(
        &self,
        account: &str,
        text: &str,
        video: &str,
        cover: &str,
        visibility: MomentVisibility,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_moment_content(
            "desktop",
            account,
            text,
            &[cover.into()],
            Some(video),
            visibility,
            key,
        )
    }
    pub fn create_video_moment(
        &self,
        p: &Principal,
        account: &str,
        text: &str,
        video: &str,
        cover: &str,
        visibility: MomentVisibility,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("moments:publish")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            "",
            key,
            crate::outbox::ClientContent::MomentVideo(text, video, cover, visibility),
        )? {
            return Ok(existing);
        }
        let draft = self.prepare_moment_content(
            &p.id,
            account,
            text,
            &[cover.into()],
            Some(video),
            visibility,
            key,
        )?;
        self.finish_client_send(p, draft)
    }
    fn prepare_moment(
        &self,
        creator: &str,
        account: &str,
        text: &str,
        images: &[String],
        visibility: MomentVisibility,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_moment_content(creator, account, text, images, None, visibility, key)
    }
    fn prepare_moment_content(
        &self,
        creator: &str,
        account: &str,
        text: &str,
        images: &[String],
        video: Option<&str>,
        visibility: MomentVisibility,
        key: &str,
    ) -> AppResult<SendStatus> {
        if !valid_moment_text(text, images.len())
            || !(16..=128).contains(&key.len())
            || !key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "动态最多10000个UTF-8字节和9张图片，正文与图片不能同时为空",
            ));
        }
        let _guard = self
            .send_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "发送任务正在处理"))?;
        let (identity, _) = self.checked_send_context(account, "filehelper")?;
        let (context, revision) = self.checked_send_context(account, &identity.sender_id)?;
        if context.sender_id != identity.sender_id || context.target_id != context.sender_id {
            return Err(AppError::new("CONNECTION_CHANGED", "当前登录账号已变化"));
        }
        let assets = images
            .iter()
            .map(|id| {
                let asset = self.repository.image(account, id)?.asset;
                let image = ImageSendAsset {
                    asset_id: id.clone(),
                    source_sha256: asset.source_sha256,
                    png_sha256: asset.sha256,
                    width: asset.image.width,
                    height: asset.image.height,
                };
                drop(self.prepared_image(account, &image)?);
                Ok(image)
            })
            .collect::<AppResult<Vec<_>>>()?;
        let video = video
            .map(|id| self.video_preview_local(account, id))
            .transpose()?;
        if video.is_some() && assets.len() != 1 {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "视频动态需要一张封面，不可混合多图",
            ));
        }
        self.persist_send(
            creator,
            account,
            key,
            revision,
            TextSendPayload {
                context,
                text: text.into(),
                moment: Some(MomentPublish {
                    visibility,
                    interaction: None,
                    images: assets,
                    video,
                }),
                mentions: Vec::new(),
                quote: None,
                voice: None,
                image: None,
                file: None,
                video: None,
                emoticon: None,
            },
        )
    }
}
