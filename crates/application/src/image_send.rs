use crate::Application;
use sha2::{Digest, Sha256};
use werc_domain::{AppError, AppResult, Principal, outbox::*};
use zeroize::Zeroizing;
impl Application {
    pub fn create_image_send_local(
        &self,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_image_send("desktop", account, target, asset_id, key)
    }
    pub fn create_image_send(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("messages:send_image")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) = self.retry_client_send(
            p,
            account,
            target,
            key,
            crate::outbox::ClientContent::Asset(3, asset_id),
        )? {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, target)?;
        let draft = self.prepare_image_send(&p.id, account, target, asset_id, key)?;
        self.finish_client_send(p, draft)
    }
    fn prepare_image_send(
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
        let asset = self.repository.image(account, asset_id)?.asset;
        let image = ImageSendAsset {
            asset_id: asset_id.into(),
            source_sha256: asset.source_sha256,
            png_sha256: asset.sha256,
            width: asset.image.width,
            height: asset.image.height,
        };
        drop(self.prepared_image(account, &image)?);
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
                image: Some(image),
                file: None,
                video: None,
                emoticon: None,
            },
        )
    }
    pub(crate) fn prepared_image(
        &self,
        account: &str,
        image: &ImageSendAsset,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        let bytes = self.decrypt_image_local(account, &image.asset_id)?;
        let asset = self.repository.image(account, &image.asset_id)?.asset;
        let (info, canonical) = werc_domain::image::canonical_png(&bytes)?;
        if asset.source_sha256 != image.source_sha256
            || hex::encode(Sha256::digest(&bytes)) != image.png_sha256
            || info.width != image.width
            || info.height != image.height
            || canonical != bytes
        {
            return Err(AppError::new(
                "ASSET_CORRUPTED",
                "图片资产与批准摘要不符；未提交",
            ));
        }
        Ok(bytes)
    }
}
