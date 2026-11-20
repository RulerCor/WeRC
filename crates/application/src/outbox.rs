use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::panic::{AssertUnwindSafe, catch_unwind};
use uuid::Uuid;
use werc_domain::{AppError, AppResult, Principal, outbox::*};
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalProof {
    id: String,
    request_digest: String,
    context_digest: String,
    expires_at: i64,
    approver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scheduled_at: Option<i64>,
}
pub(crate) enum ClientContent<'a> {
    Text(&'a str),
    Moment(&'a str, &'a [String], MomentVisibility),
    MomentAction(&'a str, &'a MomentAction),
    MomentVideo(&'a str, &'a str, &'a str, MomentVisibility),
    Asset(u32, &'a str),
    Mention(&'a str, &'a [String]),
    Quote(&'a str, &'a str),
}
fn encode<T: Serialize>(value: &T) -> AppResult<Zeroizing<Vec<u8>>> {
    Ok(Zeroizing::new(
        serde_json::to_vec(value).map_err(|_| AppError::internal())?,
    ))
}
fn corrupt() -> AppError {
    AppError::new("OUTBOX_CORRUPTED", "发送任务认证或批准校验失败，没有提交")
}
fn payload_kind(payload: &TextSendPayload) -> AppResult<(&'static str, u32)> {
    if let Some(moment) = &payload.moment {
        if payload.quote.is_some()
            || !payload.mentions.is_empty()
            || payload.voice.is_some()
            || payload.image.is_some()
            || payload.file.is_some()
            || payload.video.is_some()
            || payload.emoticon.is_some()
            || !moment.valid(&payload.text)
            || payload.context.target_id != payload.context.sender_id
        {
            return Err(corrupt());
        }
        if moment.video.is_some() { return Ok(("video-moment-request", 12)); }
        if moment.interaction.is_some() {
            return Ok(("moment-interaction-request", 11));
        }
        return Ok(match (moment.images.is_empty(), moment.visibility) {
            (true, MomentVisibility::Private) => ("private-moment-request", 9),
            (false, MomentVisibility::Private) => ("private-image-moment-request", 10),
            (true, MomentVisibility::Public) => ("public-moment-request", 9),
            (false, MomentVisibility::Public) => ("public-image-moment-request", 10),
        });
    }

    if let Some(source) = &payload.quote {
        if !payload.mentions.is_empty()
            || payload.voice.is_some()
            || payload.image.is_some()
            || payload.file.is_some()
            || payload.video.is_some()
            || payload.emoticon.is_some()
            || !source.valid()
            || !valid_send_text(&payload.text)
            || hex::encode(Sha256::digest(source.text.as_bytes())) != source.sha256
        {
            return Err(corrupt());
        }
        return Ok(("quote-request", 8));
    }
    if !payload.mentions.is_empty() {
        if payload.voice.is_some()
            || payload.image.is_some()
            || payload.file.is_some()
            || payload.video.is_some()
            || payload.emoticon.is_some()
            || !werc_domain::mentions::valid_members(&payload.context.target_id, &payload.mentions)
            || payload.text.trim().is_empty()
            || payload.text.len() > MAX_TEXT_BYTES
            || payload.text.contains('\0')
        {
            return Err(corrupt());
        }
        return Ok(("mention-request", 7));
    }
    match (
        &payload.voice,
        &payload.image,
        &payload.file,
        &payload.video,
        &payload.emoticon,
    ) {
        (None, None, None, None, None) => Ok(("text-request", 1)),
        (Some(_), None, None, None, None) if payload.text.is_empty() => Ok(("voice-request", 2)),
        (None, Some(_), None, None, None) if payload.text.is_empty() => Ok(("image-request", 3)),
        (None, None, Some(_), None, None) if payload.text.is_empty() => Ok(("file-request", 4)),
        (None, None, None, Some(_), None) if payload.text.is_empty() => Ok(("video-request", 5)),
        (None, None, None, None, Some(_)) if payload.text.is_empty() => Ok(("emoticon-request", 6)),
        _ => Err(corrupt()),
    }
}
fn validate_text_limit(payload: &TextSendPayload) -> AppResult<()> {
    if let Some(moment) = &payload.moment {
        return if moment.valid(&payload.text) {
            Ok(())
        } else {
            Err(corrupt())
        };
    }
    if payload.voice.is_none()
        && payload.image.is_none()
        && payload.file.is_none()
        && payload.video.is_none()
        && payload.emoticon.is_none()
        && !valid_send_text(&payload.text)
    {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "文字上限10000个UTF-8字节，请编辑后重新保存草稿",
        ));
    }
    Ok(())
}
fn protection(kind: &str, account: &str, id: &str) -> String {
    format!("WeRC/outbox-{kind}/v1/{account}/{id}")
}
fn scoped(principal: &Principal, account: &str) -> AppResult<()> {
    principal.require("messages:send")?;
    principal.require("operations:create")?;
    principal.require_account(account)
}
fn send_permission(version: u32) -> &'static str {
    match version {
        2 => "messages:send_voice",
        3 => "messages:send_image",
        4 => "messages:send_file",
        5 => "messages:send_video",
        6 => "messages:send_emoticon",
        9 | 10 | 11 | 12 => "moments:publish",
        _ => "messages:send",
    }
}
impl Application {
    fn outbox_secret(&self, account: &str) -> AppResult<Zeroizing<Vec<u8>>> {
        let context = format!("WeRC/outbox-index/v1/{account}");
        let wrapped = if let Some(key) = self.repository.send_secret(account, None)? {
            key
        } else {
            if !self.repository.sends(account)?.is_empty()
                || self.repository.receive_cursor(account)?.is_some()
                || self.repository.native_identity(account)?.is_some()
                || !self.repository.recovery_summaries(account)?.is_empty()
            {
                return Err(corrupt());
            }
            let mut key = Zeroizing::new([0u8; 32]);
            getrandom::fill(key.as_mut()).map_err(|_| AppError::internal())?;
            let wrapped = self.provider.protect_key(&context, key.as_ref())?;
            self.repository
                .send_secret(account, Some(&wrapped))?
                .ok_or_else(corrupt)?
        };
        let key = Zeroizing::new(self.provider.unprotect_key(&context, &wrapped)?);
        if key.len() != 32 {
            return Err(corrupt());
        }
        Ok(key)
    }
    pub(crate) fn send_digest<T: Serialize>(
        &self,
        account: &str,
        kind: &str,
        value: &T,
    ) -> AppResult<String> {
        let key = self.outbox_secret(account)?;
        let bytes = encode(&(kind, value))?;
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(&key).map_err(|_| AppError::internal())?;
        mac.update(&bytes);
        Ok(hex::encode(mac.finalize().into_bytes()))
    }
    pub(crate) fn checked_send_context(
        &self,
        account: &str,
        target: &str,
    ) -> AppResult<(SendContext, i64)> {
        self.checked_context(account, target, None)
    }
    fn checked_context(
        &self,
        account: &str,
        target: &str,
        receipt: Option<&SendContext>,
    ) -> AppResult<(SendContext, i64)> {
        self.ensure_current_connection(account)?;
        if target.is_empty()
            || target.len() > 256
            || target.starts_with("table:")
            || target.chars().any(char::is_control)
        {
            return Err(AppError::new(
                "INVALID_TARGET",
                "需要提供方验真的唯一收件标识",
            ));
        }
        let slot = self.current_connection_local()?;
        let context = if let Some(previous) = receipt {
            self.provider.validate_receipt_context(account, previous)?;
            previous.clone()
        } else {
            self.provider.resolve_send_context(account, target)?
        };
        if !context.identity_verified
            || !context.target_verified
            || context.sender_id.is_empty()
            || context.sender_id.len() > 256
            || context.target_id != target
        {
            return Err(AppError::new(
                "IDENTITY_UNVERIFIED",
                "当前登录者或唯一收件人尚未验真",
            ));
        }
        if context.epoch.is_empty()
            || context.epoch.len() > 256
            || context.target_display_name.len() > 1024
            || [&context.installation_sha256, &context.adapter_sha256]
                .iter()
                .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(AppError::new("PROVIDER_INVALID", "发送提供方上下文无效"));
        }
        let binding = self
            .bindings_local()?
            .into_iter()
            .find(|b| b.account_id == account)
            .ok_or_else(|| AppError::new("CONNECTION_CHANGED", "当前客户端没有绑定"))?;
        if binding.installation.sha256 != context.installation_sha256 {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "提供方与当前客户端指纹不同",
            ));
        }
        let revision = slot.revision.parse().map_err(|_| AppError::internal())?;
        if context.execution_mode == SendExecutionMode::Native {
            let identity = self.send_digest(account, "native-account", &context.sender_id)?;
            self.repository
                .bind_native_identity(account, revision, &identity)?;
        }
        Ok((context, revision))
    }
    pub fn create_text_send(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        scoped(p, account)?;
        if let Some(existing) =
            self.retry_client_send(p, account, target, key, ClientContent::Text(text))?
        {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, target)?;
        let draft = self.prepare_text_send(&p.id, account, target, text, key)?;
        self.finish_client_send(p, draft)
    }
    pub fn create_text_send_local(
        &self,
        account: &str,
        target: &str,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_text_send("desktop", account, target, text, key)
    }
    fn prepare_text_send(
        &self,
        creator: &str,
        account: &str,
        target: &str,
        text: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        if text.trim().is_empty()
            || text.len() > MAX_TEXT_BYTES
            || text.contains('\0')
            || !(16..=128).contains(&key.len())
            || !key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "文字上限10000个UTF-8字节，幂等键需16–128个可见ASCII字符",
            ));
        }
        let (context, revision) = self.checked_send_context(account, target)?;
        let payload = TextSendPayload {
            moment: None,
            mentions: Vec::new(),
            quote: None,
            context,
            text: text.into(),
            voice: None,
            image: None,
            file: None,
            video: None,
            emoticon: None,
        };
        self.persist_send(creator, account, key, revision, payload)
    }
    pub fn create_voice_send_local(
        &self,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        self.prepare_voice_send("desktop", account, target, asset_id, key)
    }
    pub fn create_voice_send(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
        asset_id: &str,
        key: &str,
    ) -> AppResult<SendStatus> {
        p.require("messages:send_voice")?;
        p.require("operations:create")?;
        p.require_account(account)?;
        if let Some(existing) =
            self.retry_client_send(p, account, target, key, ClientContent::Asset(2, asset_id))?
        {
            return Ok(existing);
        }
        self.prepare_client_target_if_needed(p, account, target)?;
        let draft = self.prepare_voice_send(&p.id, account, target, asset_id, key)?;
        self.finish_client_send(p, draft)
    }
    fn prepare_voice_send(
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
        let wav = self.decrypt_audio_local(account, asset_id)?;
        let encoded = self.provider.encode_voice(&wav)?;
        if werc_domain::voice::silk_duration(&encoded.bytes)? != encoded.duration_ms
            || encoded.codec_sha256.len() != 64
        {
            return Err(AppError::new("PROVIDER_INVALID", "语音编码结果不匹配"));
        }
        let payload = TextSendPayload {
            moment: None,
            mentions: Vec::new(),
            quote: None,
            context,
            text: String::new(),
            image: None,
            file: None,
            video: None,
            emoticon: None,
            voice: Some(VoiceSendAsset {
                asset_id: asset_id.into(),
                source_sha256: hex::encode(Sha256::digest(&wav)),
                silk_sha256: hex::encode(Sha256::digest(&encoded.bytes)),
                codec_sha256: encoded.codec_sha256,
                duration_ms: encoded.duration_ms,
            }),
        };
        self.persist_send(creator, account, key, revision, payload)
    }
    fn prepared_voice(
        &self,
        account: &str,
        voice: &VoiceSendAsset,
    ) -> AppResult<werc_domain::voice::EncodedVoice> {
        let wav = self.decrypt_audio_local(account, &voice.asset_id)?;
        if hex::encode(Sha256::digest(&wav)) != voice.source_sha256 {
            return Err(corrupt());
        }
        let encoded = self.provider.encode_voice(&wav)?;
        if encoded.codec_sha256 != voice.codec_sha256
            || encoded.duration_ms != voice.duration_ms
            || werc_domain::voice::silk_duration(&encoded.bytes)? != voice.duration_ms
            || hex::encode(Sha256::digest(&encoded.bytes)) != voice.silk_sha256
        {
            return Err(corrupt());
        }
        Ok(encoded)
    }
    fn require_send_features(&self, payload: &TextSendPayload) -> AppResult<()> {
        let mut ids=Vec::new();
        if !payload.mentions.is_empty() {ids.push("messages.send_mentions");}
        if payload.quote.is_some() {ids.push("messages.send_quote");}
        if let Some(moment)=&payload.moment {
            if let Some(action)=&moment.interaction {
                ids.push("moments.interact");
                if matches!(action.action,MomentActionKind::Reply) {ids.push("moments.reply");}
            }else{ids.push("moments.publish");}
        }
        if !ids.is_empty() {
            for capability in self.provider.capabilities() {
                if ids.contains(&capability.id.as_str()) && matches!(capability.availability,
                    werc_domain::capability::Availability::Blocked|werc_domain::capability::Availability::Unsupported) {
                    return Err(AppError::new("NATIVE_FEATURE_UNAVAILABLE",&capability.reason));
                }
            }
        }
        Ok(())
    }
    pub(crate) fn persist_send(
        &self,
        creator: &str,
        account: &str,
        key: &str,
        revision: i64,
        payload: TextSendPayload,
    ) -> AppResult<SendStatus> {
        let (kind, version) = payload_kind(&payload)?;
        self.require_send_features(&payload)?;
        let now = Utc::now().timestamp();
        let metadata = SendMetadata {
            schema_version: version,
            id: Uuid::new_v4().to_string(),
            account_id: account.into(),
            creator_id: creator.into(),
            idempotency_digest: self.send_digest(account, "idempotency", &(creator, key))?,
            request_digest: self.send_digest(account, kind, &payload)?,
            context_digest: self.send_digest(account, "context", &payload.context)?,
            connection_revision: revision,
            installation_sha256: payload.context.installation_sha256.clone(),
            created_at: now,
            expires_at: now + 3600,
        };
        let mut data_key = Zeroizing::new([0u8; 32]);
        let mut nonce = [0u8; 12];
        getrandom::fill(data_key.as_mut()).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut nonce).map_err(|_| AppError::internal())?;
        let wrapped_key = self.provider.protect_key(
            &protection("payload", account, &metadata.id),
            data_key.as_ref(),
        )?;
        let cipher =
            Aes256Gcm::new_from_slice(data_key.as_ref()).map_err(|_| AppError::internal())?;
        let aad = encode(&metadata)?;
        let bytes = encode(&payload)?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &bytes,
                    aad: &aad,
                },
            )
            .map_err(|_| AppError::internal())?;
        self.repository.create_send(&ProtectedSend {
            metadata,
            wrapped_key,
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }
    fn decrypt_send(&self, account: &str, id: &str) -> AppResult<(SendMetadata, TextSendPayload)> {
        let data = self.repository.send_payload(account, id)?;
        if data.nonce.len() != 12 || data.ciphertext.len() > 128 * 1024 {
            return Err(corrupt());
        }
        let key = Zeroizing::new(
            self.provider
                .unprotect_key(&protection("payload", account, id), &data.wrapped_key)?,
        );
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| corrupt())?;
        let aad = encode(&data.metadata)?;
        let bytes = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&data.nonce),
                    Payload {
                        msg: &data.ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| corrupt())?,
        );
        let payload: TextSendPayload = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
        let (kind, version) = payload_kind(&payload)?;
        if data.metadata.schema_version != version {
            return Err(corrupt());
        }
        if self.send_digest(account, kind, &payload)? != data.metadata.request_digest
            || self.send_digest(account, "context", &payload.context)?
                != data.metadata.context_digest
        {
            return Err(corrupt());
        }
        Ok((data.metadata, payload))
    }
    fn schedule_deadline(
        &self,
        account: &str,
        metadata: &SendMetadata,
        context: &SendContext,
    ) -> Option<i64> {
        let limit = if context.target_id == "filehelper" || context.target_id == context.sender_id {
            metadata.expires_at
        } else {
            // A schedule never silently renews the short native recipient lease.
            let recipient = self
                .provider
                .prepared_recipients(account)
                .ok()?
                .into_iter()
                .find(|r| r.target_id == context.target_id)?;
            metadata.expires_at.min(recipient.expires_at?)
        };
        (limit > Utc::now().timestamp()).then_some(limit)
    }
    pub fn send_preview_local(&self, account: &str, id: &str) -> AppResult<SendPreview> {
        self.ensure_account(account)?;
        let status = self.repository.send_status(account, id)?;
        let (m, payload) = self.decrypt_send(account, id)?;
        if m != status.metadata {
            return Err(corrupt());
        }
        let schedule_before = self.schedule_deadline(account, &m, &payload.context);
        let local_record = self.send_local_record(&status, &payload)?;
        Ok(SendPreview {
            schedule_before,
            local_record,
            status,
            payload,
        })
    }
    pub fn sends_local(&self, account: &str) -> AppResult<Vec<SendStatus>> {
        self.ensure_account(account)?;
        self.repository.sends(account)
    }
    pub fn send_history_page_local(
        &self,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SendPage> {
        self.ensure_account(account)?;
        self.repository.send_history_page(account, before, limit)
    }
    pub fn send_history_page(
        &self,
        p: &Principal,
        account: &str,
        before: Option<i64>,
        limit: u32,
    ) -> AppResult<SendPage> {
        p.require("operations:read")?;
        p.require_account(account)?;
        self.send_history_page_local(account, before, limit)
    }
    pub fn sends(&self, p: &Principal, account: &str) -> AppResult<Vec<SendStatus>> {
        p.require("operations:read")?;
        p.require_account(account)?;
        self.sends_local(account)
    }
    pub fn send_status(&self, p: &Principal, account: &str, id: &str) -> AppResult<SendStatus> {
        p.require("operations:read")?;
        p.require_account(account)?;
        self.repository.send_status(account, id)
    }
    pub fn send_local_record_status(&self, p:&Principal, account:&str, id:&str) -> AppResult<Option<LocalRecordStatus>> {
        let status=self.send_status(p,account,id)?;
        let (metadata,payload)=self.decrypt_send(account,id)?;
        if metadata!=status.metadata {return Err(corrupt());}
        self.send_local_record(&status,&payload)
    }
    pub fn approve_text_send_local(
        &self,
        account: &str,
        id: &str,
        expected_digest: &str,
        confirmed: bool,
    ) -> AppResult<SendStatus> {
        self.approve_timed_send_local(account, id, expected_digest, confirmed, None)
    }
    pub fn approve_timed_send_local(
        &self,
        account: &str,
        id: &str,
        expected_digest: &str,
        confirmed: bool,
        scheduled_at: Option<i64>,
    ) -> AppResult<SendStatus> {
        if !confirmed {
            return Err(AppError::new(
                "CONSENT_REQUIRED",
                "需本机确认准确收件人、文字及发送方式",
            ));
        }
        self.approve_send_as(account, id, expected_digest, scheduled_at, "desktop")
    }
    /// A client granted execution once can submit its own immutable request.
    /// Repeated requests return the existing state, including Unknown.
    pub fn submit_client_send(
        &self,
        principal: &Principal,
        account: &str,
        id: &str,
    ) -> AppResult<SendStatus> {
        principal.require("messages:execute")?;
        principal.require("operations:create")?;
        principal.require_account(account)?;
        let status = self.repository.send_status(account, id)?;
        if status.metadata.creator_id != principal.id {
            return Err(AppError::new("FORBIDDEN", "只能提交本客户端创建的请求"));
        }
        principal.require(send_permission(status.metadata.schema_version))?;
        if status.state != SendState::AwaitingApproval {
            return Ok(status);
        }
        let approved = self.approve_send_as(
            account,
            id,
            &status.metadata.request_digest,
            None,
            &format!("client:{}", principal.id),
        );
        // Concurrent retries may have approved the same immutable request.
        match approved {
            Err(error) if error.code == "INVALID_TRANSITION" => {
                self.repository.send_status(account, id)
            }
            other => other,
        }
    }
    pub(crate) fn finish_client_send(
        &self,
        principal: &Principal,
        draft: SendStatus,
    ) -> AppResult<SendStatus> {
        if principal
            .scopes
            .iter()
            .any(|scope| scope == "messages:execute")
        {
            self.submit_client_send(principal, &draft.metadata.account_id, &draft.metadata.id)
        } else {
            Ok(draft)
        }
    }
    pub(crate) fn retry_client_send(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
        key: &str,
        content: ClientContent<'_>,
    ) -> AppResult<Option<SendStatus>> {
        if !p.scopes.iter().any(|scope| scope == "messages:execute") {
            return Ok(None);
        }
        if !(16..=128).contains(&key.len()) || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "幂等键需16–128个可见ASCII字符",
            ));
        }
        let digest = self.send_digest(account, "idempotency", &(&p.id, key))?;
        let Some(existing) = self
            .repository
            .send_by_idempotency(account, &p.id, &digest)?
        else {
            return Ok(None);
        };
        let (_, payload) = self.decrypt_send(account, &existing.metadata.id)?;
        let same_target = if matches!(
            &content,
            ClientContent::Moment(..) | ClientContent::MomentAction(..) | ClientContent::MomentVideo(..)
        ) {
            payload.context.target_id == payload.context.sender_id
        } else {
            payload.context.target_id == target
        };
        let same = same_target
            && match content {
                ClientContent::MomentVideo(text, video, cover, visibility) => {
                    existing.metadata.schema_version == 12 && payload.text == text
                        && payload.moment.as_ref().is_some_and(|m| m.visibility == visibility
                            && m.video.as_ref().is_some_and(|v| v.file.asset_id == video)
                            && m.images.len() == 1 && m.images[0].asset_id == cover)
                }
                ClientContent::MomentAction(text, action) => {
                    existing.metadata.schema_version == 11
                        && payload.text == text
                        && payload.moment.as_ref().and_then(|m| m.interaction.as_ref())
                            == Some(action)
                }
                ClientContent::Moment(text, images, visibility) => {
                    matches!(existing.metadata.schema_version, 9 | 10)
                        && payload.text == text
                        && payload.moment.as_ref().is_some_and(|m| {
                            m.visibility == visibility
                                && m.images
                                    .iter()
                                    .map(|i| i.asset_id.as_str())
                                    .eq(images.iter().map(String::as_str))
                        })
                }
                ClientContent::Text(text) => {
                    existing.metadata.schema_version == 1 && payload.text == text
                }
                ClientContent::Mention(text, members) => {
                    existing.metadata.schema_version == 7
                        && payload.text == text
                        && payload.mentions == members
                }
                ClientContent::Quote(text, server_id) => {
                    existing.metadata.schema_version == 8
                        && payload.text == text
                        && payload
                            .quote
                            .as_ref()
                            .is_some_and(|q| q.server_id == server_id)
                }
                ClientContent::Asset(version, id) => {
                    existing.metadata.schema_version == version
                        && match version {
                            2 => payload.voice.as_ref().is_some_and(|a| a.asset_id == id),
                            3 => payload.image.as_ref().is_some_and(|a| a.asset_id == id),
                            4 => payload.file.as_ref().is_some_and(|a| a.asset_id == id),
                            5 => payload
                                .video
                                .as_ref()
                                .is_some_and(|a| a.file.asset_id == id),
                            6 => payload
                                .emoticon
                                .as_ref()
                                .is_some_and(|a| a.file.asset_id == id),
                            _ => false,
                        }
                }
            };
        if !same {
            return Err(AppError::new(
                "IDEMPOTENCY_CONFLICT",
                "同一幂等键已用于不同内容或收件人",
            ));
        }
        self.finish_client_send(p, existing).map(Some)
    }
    pub(crate) fn prepare_client_target_if_needed(
        &self,
        p: &Principal,
        account: &str,
        target: &str,
    ) -> AppResult<()> {
        if !p.scopes.iter().any(|scope| scope == "messages:execute") {
            return Ok(());
        }
        p.require_account(account)?;
        if let Err(error) = self.checked_send_context(account, target) {
            if error.code != "NATIVE_TARGET_NOT_VERIFIED" {
                return Err(error);
            }
            // The Bot supplies the exact username. No local message view or click
            // is required, and nickname guessing is deliberately not performed.
            self.provider.prepare_client_recipient(account, target)?;
            self.checked_send_context(account, target)?;
        }
        Ok(())
    }
    fn approve_send_as(
        &self,
        account: &str,
        id: &str,
        expected_digest: &str,
        scheduled_at: Option<i64>,
        approver: &str,
    ) -> AppResult<SendStatus> {
        let (metadata, payload) = self.decrypt_send(account, id)?;
        validate_text_limit(&payload)?;
        if metadata.request_digest != expected_digest {
            return Err(AppError::new("APPROVAL_MISMATCH", "待批准内容已变化"));
        }
        let (current, revision) = self.checked_send_context(account, &payload.context.target_id)?;
        if current != payload.context || revision != metadata.connection_revision {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "身份、目标或提供方变化，旧预览不能批准",
            ));
        }
        let now = Utc::now().timestamp();
        if scheduled_at.is_some_and(|at| at <= now || at >= metadata.expires_at) {
            return Err(AppError::new(
                "SCHEDULE_INVALID",
                "请选择未来且早于草稿到期的发送时间；草稿从创建起最多保留一小时",
            ));
        }
        let schedule_limit = if scheduled_at.is_some() {
            self.schedule_deadline(account, &metadata, &current)
                .ok_or_else(|| {
                    AppError::new("SCHEDULE_INVALID", "收件人选择已失效，请重新选择后创建草稿")
                })?
        } else {
            metadata.expires_at
        };
        if scheduled_at.is_some_and(|at| at >= schedule_limit) {
            return Err(AppError::new(
                "SCHEDULE_INVALID",
                "计划时间超过当前收件人的有效期，请缩短延迟或重新选择收件人",
            ));
        }
        let until = scheduled_at
            .unwrap_or(now)
            .saturating_add(300)
            .min(schedule_limit);
        let proof = encode(&ApprovalProof {
            id: id.into(),
            request_digest: metadata.request_digest.clone(),
            context_digest: metadata.context_digest.clone(),
            expires_at: until,
            approver: approver.into(),
            scheduled_at,
        })?;
        let proof = self
            .provider
            .protect_key(&protection("approval", account, id), &proof)?;
        self.repository.approve_timed_send(
            account,
            id,
            expected_digest,
            &metadata.context_digest,
            &SendTiming {
                scheduled_at,
                expires_at: until,
            },
            &proof,
        )
    }
    pub fn cancel_send_local(&self, account: &str, id: &str) -> AppResult<SendStatus> {
        self.ensure_account(account)?;
        self.repository.cancel_send(account, id)
    }
    pub fn cancel_send(&self, p: &Principal, account: &str, id: &str) -> AppResult<SendStatus> {
        p.require("operations:create")?;
        p.require_account(account)?;
        let status = self.repository.send_status(account, id)?;
        p.require(match status.metadata.schema_version {
            2 => "messages:send_voice",
            3 => "messages:send_image",
            4 => "messages:send_file",
            5 => "messages:send_video",
            6 => "messages:send_emoticon",
            9 | 10 | 11 | 12 => "moments:publish",
            _ => "messages:send",
        })?;
        if status.metadata.creator_id != p.id {
            return Err(AppError::new("FORBIDDEN", "只能取消本令牌创建的待发任务"));
        }
        self.cancel_send_local(account, id)
    }
    pub fn recover_sends_local(&self) -> AppResult<usize> {
        self.repository.recover_sends()
    }
    fn submission<'a>(
        &self,
        status: &'a SendStatus,
        payload: &'a TextSendPayload,
    ) -> AppResult<TextSubmission<'a>> {
        Ok(TextSubmission {
            outbox_id: &status.metadata.id,
            attempt_id: status.attempt_id.as_deref().ok_or_else(corrupt)?,
            request_digest: &status.metadata.request_digest,
            context_digest: &status.metadata.context_digest,
            context: &payload.context,
            text: &payload.text,
            deadline_unix: status.approval_expires_at.ok_or_else(corrupt)?,
        })
    }
    fn save_send_receipt(
        &self,
        status: &SendStatus,
        receipt: SendReceipt,
    ) -> AppResult<SendStatus> {
        if receipt.outbox_id != status.metadata.id
            || Some(receipt.attempt_id.as_str()) != status.attempt_id.as_deref()
            || receipt.context_digest != status.metadata.context_digest
            || receipt
                .provider_reference
                .as_ref()
                .is_some_and(|s| s.len() > 4096)
        {
            return Err(AppError::new(
                "RECEIPT_MISMATCH",
                "提供方回执与提交不匹配，结果保持未知",
            ));
        }
        let bytes = encode(&receipt)?;
        let protected = self.provider.protect_key(
            &protection("receipt", &status.metadata.account_id, &status.metadata.id),
            &bytes,
        )?;
        self.repository.record_send_receipt(
            &status.metadata.account_id,
            &ProtectedReceipt {
                outbox_id: receipt.outbox_id,
                attempt_id: receipt.attempt_id,
                context_digest: receipt.context_digest,
                evidence: receipt.evidence,
                protected,
            },
        )
    }
    pub fn execute_send_local(&self, account: &str, id: &str) -> AppResult<SendStatus> {
        let _guard = self
            .send_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "当前发送正在提交或回查"))?;
        let ready = (|| {
            let status = self.repository.send_status(account, id)?;
            if status.state != SendState::Queued {
                return Err(AppError::new(
                    "INVALID_TRANSITION",
                    "只有已批准排队任务可以提交一次",
                ));
            }
            if status
                .scheduled_at
                .is_some_and(|at| at > Utc::now().timestamp())
            {
                return Err(AppError::new(
                    "SEND_NOT_DUE",
                    "尚未到计划发送时间，任务保持排队",
                ));
            }
            let (metadata, payload) = self.decrypt_send(account, id)?;
            validate_text_limit(&payload)?;
            self.require_send_features(&payload)?;
            let (current, revision) =
                self.checked_send_context(account, &payload.context.target_id)?;
            if current != payload.context || revision != metadata.connection_revision {
                return Err(AppError::new(
                    "CONNECTION_CHANGED",
                    "批准后身份或提供方已变化，没有提交",
                ));
            }
            let proof = self
                .repository
                .send_approval(account, id)?
                .ok_or_else(corrupt)?;
            let proof = Zeroizing::new(
                self.provider
                    .unprotect_key(&protection("approval", account, id), &proof)?,
            );
            let proof: ApprovalProof = serde_json::from_slice(&proof).map_err(|_| corrupt())?;
            if metadata != status.metadata
                || proof.id != id
                || proof.request_digest != metadata.request_digest
                || proof.context_digest != metadata.context_digest
                || Some(proof.expires_at) != status.approval_expires_at
                || proof.scheduled_at != status.scheduled_at
                || (proof.approver != "desktop"
                    && proof.approver != format!("client:{}", metadata.creator_id))
            {
                return Err(corrupt());
            }
            if proof.approver != "desktop" {
                let token = self
                    .repository
                    .tokens()?
                    .into_iter()
                    .find(|token| {
                        token.id == metadata.creator_id
                            && !token.revoked
                            && token.scopes.iter().any(|s| s == "messages:execute")
                    })
                    .ok_or_else(|| AppError::new("AUTH_REVOKED", "客户端执行授权已撤销"))?;
                if chrono::DateTime::parse_from_rfc3339(&token.expires_at)
                    .map_or(true, |expiry| expiry.timestamp() <= Utc::now().timestamp())
                {
                    return Err(AppError::new("AUTH_REVOKED", "客户端执行授权已到期"));
                }
            }
            // Re-encode the authenticated asset before consuming the one-shot attempt.
            // Deletion, codec replacement or changed samples cancel without submission.
            let voice = payload
                .voice
                .as_ref()
                .map(|v| self.prepared_voice(account, v))
                .transpose()?;
            let image = payload
                .image
                .as_ref()
                .map(|v| self.prepared_image(account, v))
                .transpose()?;
            let file = payload
                .file
                .as_ref()
                .map(|f| self.prepared_file(account, f))
                .transpose()?;
            let video = payload
                .video
                .as_ref()
                .or_else(|| payload.moment.as_ref().and_then(|m| m.video.as_ref()))
                .map(|v| self.prepared_video(account, v))
                .transpose()?;
            let emoticon = payload
                .emoticon
                .as_ref()
                .map(|e| self.prepared_emoticon(account, e))
                .transpose()?;
            let moment_images = payload
                .moment
                .as_ref()
                .map(|m| {
                    m.images
                        .iter()
                        .map(|a| self.prepared_image(account, a))
                        .collect::<AppResult<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok((
                status,
                payload,
                voice,
                image,
                file,
                video,
                emoticon,
                moment_images,
            ))
        })();
        let (prepared, payload, voice, image, file, video, emoticon, moment_images) = match ready {
            Ok(v) => v,
            Err(e) => {
                if !matches!(
                    e.code.as_str(),
                    "INVALID_TRANSITION" | "BUSY" | "SEND_NOT_DUE"
                ) {
                    let _ = self.repository.cancel_send(account, id);
                }
                return Err(e);
            }
        };
        let attempt = Uuid::new_v4().to_string();
        // 提交记录必须先落盘。此后任何异常均不得再入队，也不得自动重发。
        let status = match self.repository.start_send(&prepared, &attempt) {
            Ok(status) => status,
            Err(error) => {
                if !matches!(
                    error.code.as_str(),
                    "BUSY" | "INVALID_TRANSITION" | "SEND_NOT_DUE"
                ) {
                    let _ = self.repository.cancel_send(account, id);
                }
                return Err(error);
            }
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            let submission = self.submission(&status, &payload)?;
            if let Some(action) = payload.moment.as_ref().and_then(|m| m.interaction.as_ref()) {
                self.provider.submit_moment_action(submission, action)
            } else if let Some(moment) = &payload.moment {
                if let Some(asset) = &moment.video {
                    let bytes = video.as_ref().ok_or_else(corrupt)?;
                    return self.provider.submit_video_moment(MomentVideoSubmission {
                        submission, video: asset, bytes: &bytes.bytes,
                        cover: MomentImage { asset: &moment.images[0], png: &moment_images[0] },
                    }, moment.visibility);
                }
                let images = moment
                    .images
                    .iter()
                    .zip(&moment_images)
                    .map(|(asset, png)| MomentImage { asset, png })
                    .collect::<Vec<_>>();
                self.provider.submit_moment(
                    MomentImageSubmission {
                        submission,
                        images: &images,
                    },
                    moment.visibility,
                )
            } else if let (Some(asset), Some(contents)) = (&payload.emoticon, &emoticon) {
                self.provider.submit_emoticon(EmoticonSubmission {
                    submission,
                    emoticon: asset,
                    bytes: &contents.bytes,
                })
            } else if let (Some(asset), Some(contents)) = (&payload.video, &video) {
                self.provider.submit_video(VideoSubmission {
                    submission,
                    video: asset,
                    bytes: &contents.bytes,
                })
            } else if let (Some(asset), Some(encoded)) = (&payload.voice, &voice) {
                self.provider.submit_voice(VoiceSubmission {
                    submission,
                    voice: asset,
                    silk: &encoded.bytes,
                })
            } else if let (Some(asset), Some(contents)) = (&payload.file, &file) {
                self.provider.submit_file(FileSubmission {
                    submission,
                    file: asset,
                    bytes: &contents.bytes,
                })
            } else if let (Some(asset), Some(bytes)) = (&payload.image, &image) {
                self.provider.submit_image(ImageSubmission {
                    submission,
                    image: asset,
                    png: bytes,
                })
            } else if let Some(source) = &payload.quote {
                self.provider
                    .submit_quote(QuoteSubmission { submission, source })
            } else if !payload.mentions.is_empty() {
                self.provider.submit_mentions(MentionSubmission {
                    submission,
                    members: &payload.mentions,
                })
            } else {
                self.provider.submit_text(submission)
            }
        }));
        match result {
            Ok(Ok(receipt)) => match self.save_send_receipt(&status, receipt) {
                Ok(value) => Ok(value),
                Err(error) => {
                    self.repository.mark_send_unknown(account, id, &attempt)?;
                    Err(error)
                }
            },
            _ => self.repository.mark_send_unknown(account, id, &attempt),
        }
    }
    pub fn run_send_once_local(&self) -> AppResult<bool> {
        let Some(status) = self.repository.next_queued_send()? else {
            return Ok(false);
        };
        match self.execute_send_local(&status.metadata.account_id, &status.metadata.id) {
            Ok(_) => Ok(true),
            Err(e) if matches!(e.code.as_str(), "BUSY" | "SEND_NOT_DUE") => Ok(false),
            Err(e) => {
                let _ = self
                    .repository
                    .cancel_send(&status.metadata.account_id, &status.metadata.id);
                Err(e)
            }
        }
    }
    pub fn reconcile_send_local(&self, account: &str, id: &str) -> AppResult<SendStatus> {
        let _guard = self
            .send_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "当前发送正在提交或回查"))?;
        let status = self.repository.send_status(account, id)?;
        if !matches!(status.state, SendState::Unknown | SendState::ServerAccepted) {
            return Err(AppError::new("INVALID_TRANSITION", "此任务不需要回查"));
        }
        let (metadata, payload) = self.decrypt_send(account, id)?;
        if metadata != status.metadata {
            return Err(corrupt());
        }
        let previous = self
            .repository
            .send_receipt(account, id)?
            .map(|bytes| {
                self.provider
                    .unprotect_key(&protection("receipt", account, id), &bytes)
                    .map(Zeroizing::new)
            })
            .transpose()?;
        let previous: Option<SendReceipt> = previous
            .as_ref()
            .map(|bytes| serde_json::from_slice(bytes).map_err(|_| corrupt()))
            .transpose()?;
        if previous.as_ref().is_some_and(|r| {
            r.outbox_id != id
                || Some(r.attempt_id.as_str()) != status.attempt_id.as_deref()
                || r.context_digest != metadata.context_digest
        }) {
            return Err(corrupt());
        }
        if let Some(receipt) = self.reconcile_exact_receipt(&status, &payload)? {
            // A later local deletion cannot downgrade a previously verified server acceptance.
            if status.state == SendState::ServerAccepted
                || (previous.as_ref() == Some(&receipt)
                    && status.evidence == Some(receipt.evidence))
            {
                return Ok(status);
            }
            return self.save_send_receipt(&status, receipt);
        }
        let (current, revision) =
            self.checked_context(account, &payload.context.target_id, Some(&payload.context))?;
        if metadata != status.metadata
            || current != payload.context
            || revision != metadata.connection_revision
        {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "旧提交不能交给新身份回查或重发",
            ));
        }
        if let Some(previous) = &previous {
            self.remember_exact_receipt(&status, &payload, previous)?;
        }
        let receipt = catch_unwind(AssertUnwindSafe(|| {
            let query = TextReceiptQuery {
                outbox_id: id,
                attempt_id: status.attempt_id.as_deref().ok_or_else(corrupt)?,
                context_digest: &metadata.context_digest,
                context: &payload.context,
            };
            let reference = previous
                .as_ref()
                .and_then(|r| r.provider_reference.as_deref());
            if payload
                .moment
                .as_ref()
                .is_some_and(|m| m.interaction.is_some())
            {
                self.provider.reconcile_moment_action(query, reference)
            } else if payload.moment.is_some() {
                self.provider.reconcile_private_moment(query, reference)
            } else if payload.emoticon.is_some() {
                self.provider.reconcile_emoticon(query, reference)
            } else if payload.video.is_some() {
                self.provider.reconcile_video(query, reference)
            } else if payload.voice.is_some() {
                self.provider.reconcile_voice(query, reference)
            } else if payload.file.is_some() {
                self.provider.reconcile_file(query, reference)
            } else if payload.image.is_some() {
                self.provider.reconcile_image(query, reference)
            } else {
                self.provider.reconcile_text(query, reference)
            }
        }))
        .map_err(|_| AppError::new("PROVIDER_QUERY_FAILED", "回查异常；原结果保留，没有重发"))??;
        let saved =
            if previous.as_ref() == Some(&receipt) && status.evidence == Some(receipt.evidence) {
                status
            } else {
                self.save_send_receipt(
                    &status,
                    SendReceipt {
                        outbox_id: receipt.outbox_id.clone(),
                        attempt_id: receipt.attempt_id.clone(),
                        context_digest: receipt.context_digest.clone(),
                        evidence: receipt.evidence,
                        provider_reference: receipt.provider_reference.clone(),
                    },
                )?
            };
        self.remember_exact_receipt(&saved, &payload, &receipt)?;
        if let Some(exact) = self.reconcile_exact_receipt(&saved, &payload)? {
            if exact.evidence == SendEvidence::ServerAccepted {
                return self.save_send_receipt(&saved, exact);
            }
        }
        Ok(saved)
    }
}
