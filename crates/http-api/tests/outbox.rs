#![cfg(windows)]
#[path="support/owned_directory.rs"]
mod owned_directory;
use chrono::Utc;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppError, AppResult, capability::Capability, connection::*, outbox::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

struct SyntheticSender {
    repository: Arc<SqliteRepository>,
    context: Mutex<SendContext>,
    mode: AtomicUsize,
    calls: AtomicUsize,
    queries: AtomicUsize,
    gate: Mutex<(bool, bool)>,
    wake: Condvar,
}
impl SyntheticSender {
    fn release(&self) {
        self.gate.lock().unwrap().1 = true;
        self.wake.notify_all();
    }
    fn wait_entered(&self) {
        let (gate, timeout) = self
            .wake
            .wait_timeout_while(
                self.gate.lock().unwrap(),
                std::time::Duration::from_secs(10),
                |v| !v.0,
            )
            .unwrap();
        assert!(!timeout.timed_out() && gate.0, "自有提供方没有进入提交");
    }
    fn receipt(&self, s: &TextSubmission<'_>, evidence: SendEvidence) -> SendReceipt {
        SendReceipt {
            outbox_id: s.outbox_id.into(),
            attempt_id: s.attempt_id.into(),
            context_digest: s.context_digest.into(),
            evidence,
            provider_reference: Some("synthetic-private-provider-reference".into()),
        }
    }
}
impl CapabilityProvider for SyntheticSender {
    fn submit_moment_action(
        &self,
        s: TextSubmission<'_>,
        action: &MomentAction,
    ) -> AppResult<SendReceipt> {
        assert!(action.valid(s.text));
        assert_eq!(s.context.sender_id, s.context.target_id);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.receipt(&s, SendEvidence::NativeQueued))
    }
    fn reconcile_moment_action(
        &self,
        q: TextReceiptQuery<'_>,
        _: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        Ok(SendReceipt {
            outbox_id: q.outbox_id.into(),
            attempt_id: q.attempt_id.into(),
            context_digest: q.context_digest.into(),
            evidence: SendEvidence::NativeQueued,
            provider_reference: None,
        })
    }

    fn submit_video_moment(
        &self,
        s: MomentVideoSubmission<'_>,
        _: MomentVisibility,
    ) -> AppResult<SendReceipt> {
        use sha2::{Digest, Sha256};
        assert_eq!(
            s.submission.context.target_id,
            s.submission.context.sender_id
        );
        assert_eq!(hex::encode(Sha256::digest(s.bytes)), s.video.file.sha256);
        assert_eq!(
            werc_domain::video::inspect_send_mp4(&s.video.file.file_name, s.bytes)?,
            s.video.info
        );
        assert_eq!(
            hex::encode(Sha256::digest(s.cover.png)),
            s.cover.asset.png_sha256
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.receipt(&s.submission, SendEvidence::NativeQueued))
    }
    fn submit_moment(
        &self,
        s: MomentImageSubmission<'_>,
        visibility: MomentVisibility,
    ) -> AppResult<SendReceipt> {
        assert!(matches!(
            visibility,
            MomentVisibility::Public | MomentVisibility::Private
        ));
        if s.images.is_empty() {
            self.submit_private_moment(s.submission)
        } else {
            self.submit_private_image_moment(s)
        }
    }

    fn submit_private_image_moment(&self, s: MomentImageSubmission<'_>) -> AppResult<SendReceipt> {
        use sha2::{Digest, Sha256};
        assert_eq!(
            s.submission.context.target_id,
            s.submission.context.sender_id
        );
        let account = self.repository.current_connection()?.account_id.unwrap();
        let status = self
            .repository
            .send_status(&account, s.submission.outbox_id)?;
        assert_eq!(status.metadata.schema_version, 10);
        assert!(status.attempt_id.is_some());
        assert!((1..=9).contains(&s.images.len()));
        for image in s.images {
            let (info, bytes) = werc_domain::image::canonical_png(image.png)?;
            assert_eq!(&*bytes, image.png);
            assert_eq!(
                (info.width, info.height),
                (image.asset.width, image.asset.height)
            );
            assert_eq!(
                hex::encode(Sha256::digest(image.png)),
                image.asset.png_sha256
            );
        }
        self.submit_text(s.submission)
    }
    fn submit_private_moment(&self, s: TextSubmission<'_>) -> AppResult<SendReceipt> {
        assert_eq!(s.context.target_id, s.context.sender_id);
        let a = self.repository.current_connection()?.account_id.unwrap();
        assert_eq!(
            self.repository
                .send_status(&a, s.outbox_id)?
                .metadata
                .schema_version,
            9
        );
        self.submit_text(s)
    }
    fn reconcile_private_moment(
        &self,
        q: TextReceiptQuery<'_>,
        r: Option<&str>,
    ) -> AppResult<SendReceipt> {
        let mut receipt = self.reconcile_text(q, r)?;
        receipt.evidence = SendEvidence::ServerAccepted;
        Ok(receipt)
    }
    fn submit_video(&self, s: VideoSubmission<'_>) -> AppResult<SendReceipt> {
        assert_eq!(
            werc_domain::video::inspect_send_mp4(&s.video.file.file_name, s.bytes)?,
            s.video.info
        );
        self.submit_file(FileSubmission {
            submission: s.submission,
            file: &s.video.file,
            bytes: s.bytes,
        })
    }
    fn reconcile_video(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.reconcile_text(q, r)
    }
    fn submit_emoticon(&self, s: EmoticonSubmission<'_>) -> AppResult<SendReceipt> {
        assert_eq!(
            werc_domain::emoticon::inspect_gif(&s.emoticon.file.file_name, s.bytes)?,
            s.emoticon.info
        );
        self.submit_file(FileSubmission {
            submission: s.submission,
            file: &s.emoticon.file,
            bytes: s.bytes,
        })
    }
    fn reconcile_emoticon(
        &self,
        q: TextReceiptQuery<'_>,
        r: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.reconcile_text(q, r)
    }
    fn submit_file(&self, s: FileSubmission<'_>) -> AppResult<SendReceipt> {
        use sha2::{Digest, Sha256};
        werc_domain::file::validate_file_name(&s.file.file_name)?;
        assert_eq!(s.bytes.len(), s.file.size);
        assert_eq!(hex::encode(Sha256::digest(s.bytes)), s.file.sha256);
        let mut r = self.submit_text(s.submission)?;
        r.evidence = SendEvidence::NativeQueued;
        Ok(r)
    }
    fn reconcile_file(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.reconcile_text(q, r)
    }
    fn submit_image(&self, s: ImageSubmission<'_>) -> AppResult<SendReceipt> {
        use sha2::{Digest, Sha256};
        let (info, png) = werc_domain::image::canonical_png(s.png)?;
        assert_eq!(&*png, s.png);
        assert_eq!((info.width, info.height), (s.image.width, s.image.height));
        assert_eq!(hex::encode(Sha256::digest(s.png)), s.image.png_sha256);
        let mut r = self.submit_text(s.submission)?;
        r.evidence = SendEvidence::NativeQueued;
        Ok(r)
    }
    fn reconcile_image(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.reconcile_text(q, r)
    }
    fn encode_voice(&self, wav: &[u8]) -> AppResult<werc_domain::voice::EncodedVoice> {
        use sha2::{Digest, Sha256};
        let pcm = werc_domain::voice::voice_pcm(wav)?;
        let hash = Sha256::digest(&pcm);
        let mut bytes = b"\x02#!SILK_V3".to_vec();
        for _ in pcm.chunks(640) {
            bytes.extend_from_slice(&[2, 0, hash[0], hash[1]]);
        }
        Ok(werc_domain::voice::EncodedVoice {
            bytes: zeroize::Zeroizing::new(bytes),
            duration_ms: (pcm.len() / 32) as u32,
            codec_sha256: if self.mode.load(Ordering::SeqCst) == 10 {
                "4".repeat(64)
            } else {
                "3".repeat(64)
            },
        })
    }
    fn submit_voice(&self, s: VoiceSubmission<'_>) -> AppResult<SendReceipt> {
        use sha2::{Digest, Sha256};
        assert!(s.submission.text.is_empty());
        assert_eq!(hex::encode(Sha256::digest(s.silk)), s.voice.silk_sha256);
        assert_eq!(
            werc_domain::voice::silk_duration(s.silk)?,
            s.voice.duration_ms
        );
        let mut r = self.submit_text(s.submission)?;
        r.evidence = SendEvidence::NativeQueued;
        Ok(r)
    }
    fn reconcile_voice(&self, q: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.reconcile_text(q, r)
    }
    fn capabilities(&self) -> Vec<Capability> {
        let mut c = Capability::pending("messages.send_voice", "自有模拟语音，不访问微信");
        c.evidence = werc_domain::capability::Evidence::FixtureVerified;
        c.availability = werc_domain::capability::Availability::Ready;
        let mut image = c.clone();
        image.id = "messages.send_image".into();
        let mut file = c.clone();
        file.id = "messages.send_file".into();
        let mut video = c.clone();
        video.id = "messages.send_video".into();
        let mut emoticon = c.clone();
        emoticon.id = "messages.send_emoticon".into();
        let mut capabilities=vec![c, image, file, video, emoticon];
        if self.mode.load(Ordering::SeqCst)==6 {
            let mut blocked=Capability::pending("messages.send_mentions","当前精确构建尚未开放群提及");
            blocked.availability=werc_domain::capability::Availability::Blocked;
            capabilities.push(blocked);
        }
        capabilities
    }
    fn protect_key(&self, c: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(c, key)
    }
    fn unprotect_key(&self, c: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.unprotect_key(c, key)
    }
    fn resolve_send_context(&self, _account: &str, target: &str) -> AppResult<SendContext> {
        let mut c = self.context.lock().unwrap().clone();
        if c.target_id == c.sender_id && target == "filehelper" {
            c.target_id = target.into();
        }
        if c.target_id != target {
            return Err(AppError::new("INVALID_TARGET", "合成目标不匹配"));
        }
        Ok(c)
    }
    fn submit_text(&self, s: TextSubmission<'_>) -> AppResult<SendReceipt> {
        // 在模拟任何外部副作用前，核查尝试记录和 Submitting 已经持久提交。
        let current = self
            .repository
            .current_connection()
            .unwrap()
            .account_id
            .unwrap();
        let status = self.repository.send_status(&current, s.outbox_id).unwrap();
        assert_eq!(status.state, SendState::Submitting);
        assert_eq!(status.attempt_id.as_deref(), Some(s.attempt_id));
        if *self.context.lock().unwrap() != *s.context || Utc::now().timestamp() >= s.deadline_unix
        {
            return Err(AppError::new(
                "CONNECTION_CHANGED",
                "自有适配器拒绝迟到上下文",
            ));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode.load(Ordering::SeqCst) {
            1 => Ok(self.receipt(&s, SendEvidence::LocalAccepted)),
            2 => Err(AppError::new("SYNTHETIC_ACK_LOST", "合成ACK丢失")),
            3 => panic!("synthetic provider panic"),
            4 => {
                let mut r = self.receipt(&s, SendEvidence::ServerAccepted);
                r.attempt_id = uuid::Uuid::new_v4().to_string();
                Ok(r)
            }
            5 => {
                let mut g = self.gate.lock().unwrap();
                g.0 = true;
                self.wake.notify_all();
                let _g = self.wake.wait_while(g, |v| !v.1).unwrap();
                Ok(self.receipt(&s, SendEvidence::ServerAccepted))
            }
            _ => Ok(self.receipt(&s, SendEvidence::ServerAccepted)),
        }
    }
    fn reconcile_text(
        &self,
        s: TextReceiptQuery<'_>,
        reference: Option<&str>,
    ) -> AppResult<SendReceipt> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        if reference.is_some() {
            assert_eq!(reference, Some("synthetic-private-provider-reference"));
        }
        Ok(SendReceipt {
            outbox_id: s.outbox_id.into(),
            attempt_id: s.attempt_id.into(),
            context_digest: s.context_digest.into(),
            evidence: SendEvidence::RecipientConfirmed,
            provider_reference: reference.map(str::to_owned),
        })
    }
}

#[path = "outbox/client_execution.rs"]
mod client_execution;
#[path = "outbox/image_moments.rs"]
mod image_moments;

fn voice_wav() -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(676u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(32000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(640u32.to_le_bytes());
    bytes.extend(vec![1u8; 640]);
    bytes
}
fn voice_draft(f: &Fixture) -> (String, SendStatus) {
    let asset = f
        .application
        .upload_audio_local(&f.account, voice_wav())
        .unwrap();
    let draft = f
        .application
        .create_voice_send_local(
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            "synthetic-voice-once-0001",
        )
        .unwrap();
    (asset.id, draft)
}
#[test]
fn image_assets_are_encrypted_and_approved_attempt_is_never_repeated() {
    let f = Fixture::new();
    let original = include_bytes!("../../../tests/fixtures/owned-image.png").to_vec();
    let asset = f
        .application
        .upload_image_local(&f.account, original.clone())
        .unwrap();
    assert_eq!((asset.image.width, asset.image.height), (2, 2));
    let png = f
        .application
        .image_preview_local(&f.account, &asset.id)
        .unwrap();
    assert_ne!(&*png, original.as_slice());
    assert!(!png.windows(7).any(|w| w == b"Comment"));
    let create = || {
        f.application
            .create_image_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-image-idempotency-0001",
            )
            .unwrap()
    };
    let draft = create();
    assert_eq!(draft.metadata.schema_version, 3);
    assert_eq!(create().metadata.id, draft.metadata.id);
    assert_eq!(draft.state, SendState::AwaitingApproval);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    let preview = f
        .application
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(
        preview.payload.image.as_ref().unwrap().png_sha256,
        asset.sha256
    );
    assert!(preview.payload.voice.is_none() && preview.payload.text.is_empty());
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(!disk.windows(png.len()).any(|w| w == png.as_slice()));
    f.approve(&draft);
    let sent = f
        .application
        .execute_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(sent.state, SendState::Unknown);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn image_expiry_clears_asset_and_cancels_approved_draft_before_native_submission() {
    let f = Fixture::new();
    let asset = f
        .application
        .upload_image_local(
            &f.account,
            include_bytes!("../../../tests/fixtures/owned-image.png").to_vec(),
        )
        .unwrap();
    let draft = f
        .application
        .create_image_send_local(
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            "owned-image-expiry-0001",
        )
        .unwrap();
    f.approve(&draft);
    let c = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
    c.execute(
        "UPDATE image_assets SET expires_at='2000-01-01T00:00:00+00:00' WHERE id=?1",
        [&asset.id],
    )
    .unwrap();
    drop(c);
    assert!(
        f.application
            .image_assets_local(&f.account)
            .unwrap()
            .is_empty()
    );
    assert!(
        f.application
            .image_preview_local(&f.account, &asset.id)
            .is_err()
    );
    assert_eq!(f.repository.purge_expired_images().unwrap(), 1);
    assert_eq!(f.repository.purge_expired_images().unwrap(), 0);
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.repository
            .send_status(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Canceled
    );
}

#[test]
fn image_deletion_corruption_and_context_change_cancel_before_submission() {
    for mode in 0..4 {
        let f = Fixture::new();
        let asset = f
            .application
            .upload_image_local(
                &f.account,
                include_bytes!("../../../tests/fixtures/owned-image.png").to_vec(),
            )
            .unwrap();
        let draft = f
            .application
            .create_image_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-image-guard-0001",
            )
            .unwrap();
        f.approve(&draft);
        match mode {
            0 => f
                .application
                .delete_image_local(&f.account, &asset.id)
                .unwrap(),
            1 => {
                f.database()
                    .execute_batch(
                        "UPDATE image_assets SET ciphertext=zeroblob(length(ciphertext))",
                    )
                    .unwrap();
            }
            2 => {
                f.provider.context.lock().unwrap().epoch = "new-context".into();
            }
            _ => {
                f.database()
                    .execute_batch(
                        "UPDATE image_assets SET metadata=json_set(metadata,'$.image.width',3)",
                    )
                    .unwrap();
            }
        }
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        let status = f.application.sends_local(&f.account).unwrap().remove(0);
        assert_eq!(status.state, SendState::Canceled);
        assert!(status.attempt_id.is_none());
    }
}

#[test]
fn voice_uses_the_existing_one_shot_outbox_and_never_promotes_native_acceptance() {
    let f = Fixture::new();
    let (asset, draft) = voice_draft(&f);
    let preview = f
        .application
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert!(preview.payload.text.is_empty());
    assert_eq!(preview.payload.voice.as_ref().unwrap().duration_ms, 20);
    assert_eq!(draft.metadata.schema_version, 2);
    let repeated = f
        .application
        .create_voice_send_local(
            &f.account,
            "synthetic-private-recipient-identity",
            &asset,
            "synthetic-voice-once-0001",
        )
        .unwrap();
    assert_eq!(repeated.metadata.id, draft.metadata.id);
    f.approve(&draft);
    let result = f
        .application
        .execute_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(result.state, SendState::Unknown);
    assert_eq!(result.evidence, Some(SendEvidence::NativeQueued));
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    f.application
        .delete_audio_local(&f.account, &asset)
        .unwrap();
    f.application
        .reconcile_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn voice_asset_deletion_or_codec_change_cancels_before_attempt() {
    for change in 0..2 {
        let f = Fixture::new();
        let (asset, draft) = voice_draft(&f);
        f.approve(&draft);
        if change == 0 {
            f.application
                .delete_audio_local(&f.account, &asset)
                .unwrap();
        } else {
            f.provider.mode.store(10, Ordering::SeqCst);
        }
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        let status = f
            .repository
            .send_status(&f.account, &draft.metadata.id)
            .unwrap();
        assert_eq!(status.state, SendState::Canceled);
        assert!(status.attempt_id.is_none());
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn voice_lost_ack_does_not_reencode_or_resubmit_unknown() {
    let f = Fixture::new();
    let (_, draft) = voice_draft(&f);
    f.approve(&draft);
    f.provider.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    f.application.recover_sends_local().unwrap();
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn legacy_text_authentication_is_unchanged_and_voice_format_is_checked_before_draft() {
    let f = Fixture::new();
    let draft = f.draft("legacy-text-serialization-001");
    let preview = f
        .application
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert!(
        !serde_json::to_value(&preview.payload)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("voice")
    );
    let mut wav = voice_wav();
    wav[24..28].copy_from_slice(&8000u32.to_le_bytes());
    wav[28..32].copy_from_slice(&16000u32.to_le_bytes());
    let asset = f.application.upload_audio_local(&f.account, wav).unwrap();
    assert!(
        f.application
            .create_voice_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "invalid-voice-format-001"
            )
            .is_err()
    );
    assert_eq!(f.repository.sends(&f.account).unwrap().len(), 1);
}

#[tokio::test]
async fn voice_api_requires_its_own_scope_and_never_approves_or_sends() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let asset = f
        .application
        .upload_audio_local(&f.account, voice_wav())
        .unwrap();
    let issue = |scope: &str| {
        f.application
            .issue_token_local(
                "自有语音API",
                vec![f.account.clone()],
                vec![
                    scope.into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let text = issue("messages:send");
    let voice = issue("messages:send_voice");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, method: &str, uri: &str, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "owned-voice-api-0001")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let input = serde_json::json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","asset_id":asset.id});
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &text.secret,
                "POST",
                "/api/v1/messages/send-voice",
                input.clone()
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router
        .clone()
        .oneshot(request(
            &voice.secret,
            "POST",
            "/api/v1/messages/send-voice",
            input,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["state"], "awaiting_approval");
    assert!(!body.to_string().contains(&asset.id));
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let id = body["data"]["metadata"]["id"].as_str().unwrap();
    let uri = format!("/api/v1/outbox/{id}?account_id={}", f.account);
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &voice.secret,
                "DELETE",
                &uri,
                serde_json::Value::Null
            ))
            .await
            .unwrap()
            .status(),
        200
    );
    let principal = f.application.authenticate(&voice.secret).unwrap();
    let draft = f
        .application
        .create_voice_send(
            &principal,
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            "owned-voice-api-0002",
        )
        .unwrap();
    f.approve(&draft);
    f.application.revoke_token_local(&voice.info.id).unwrap();
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn image_api_requires_its_own_scope_and_never_approves_or_sends() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let asset = f
        .application
        .upload_image_local(
            &f.account,
            include_bytes!("../../../tests/fixtures/owned-image.png").to_vec(),
        )
        .unwrap();
    let issue = |scope: &str| {
        f.application
            .issue_token_local(
                "自有图片API",
                vec![f.account.clone()],
                vec![
                    scope.into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let text = issue("messages:send");
    let image = issue("messages:send_image");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, method: &str, uri: &str, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "owned-image-api-0001")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let input = serde_json::json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","asset_id":asset.id});
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &text.secret,
                "POST",
                "/api/v1/messages/send-image",
                input.clone()
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router
        .clone()
        .oneshot(request(
            &image.secret,
            "POST",
            "/api/v1/messages/send-image",
            input,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["state"], "awaiting_approval");
    assert!(!body.to_string().contains(&asset.id));
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let id = body["data"]["metadata"]["id"].as_str().unwrap();
    let uri = format!("/api/v1/outbox/{id}?account_id={}", f.account);
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &image.secret,
                "DELETE",
                &uri,
                serde_json::Value::Null
            ))
            .await
            .unwrap()
            .status(),
        200
    );
    let principal = f.application.authenticate(&image.secret).unwrap();
    let draft = f
        .application
        .create_image_send(
            &principal,
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            "owned-image-api-0002",
        )
        .unwrap();
    f.approve(&draft);
    f.application.revoke_token_local(&image.info.id).unwrap();
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn file_api_requires_its_own_scope_and_never_approves_or_sends() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let asset = f
        .application
        .upload_file_local(
            &f.account,
            "自有附件.txt",
            b"owned attachment bytes".to_vec(),
        )
        .unwrap();
    let issue = |scope: &str| {
        f.application
            .issue_token_local(
                "自有文件API",
                vec![f.account.clone()],
                vec![
                    scope.into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let text = issue("messages:send");
    let file = issue("messages:send_file");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, method: &str, uri: &str, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "owned-file-api-0001")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let input = serde_json::json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","asset_id":asset.id});
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &text.secret,
                "POST",
                "/api/v1/messages/send-file",
                input.clone()
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router
        .clone()
        .oneshot(request(
            &file.secret,
            "POST",
            "/api/v1/messages/send-file",
            input,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["state"], "awaiting_approval");
    assert!(!body.to_string().contains(&asset.id));
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let id = body["data"]["metadata"]["id"].as_str().unwrap();
    let uri = format!("/api/v1/outbox/{id}?account_id={}", f.account);
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &file.secret,
                "DELETE",
                &uri,
                serde_json::Value::Null
            ))
            .await
            .unwrap()
            .status(),
        200
    );
    let principal = f.application.authenticate(&file.secret).unwrap();
    let draft = f
        .application
        .create_file_send(
            &principal,
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            "owned-file-api-0002",
        )
        .unwrap();
    f.approve(&draft);
    f.application.revoke_token_local(&file.info.id).unwrap();
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn file_names_and_bytes_are_encrypted_and_unknown_is_never_replayed() {
    let f = Fixture::new();
    let bytes = b"owned attachment content\n";
    let name = "私有自有文件.txt";
    let asset = f
        .application
        .upload_file_local(&f.account, name, bytes.to_vec())
        .unwrap();
    let view = f
        .application
        .file_preview_local(&f.account, &asset.id)
        .unwrap();
    assert_eq!(view.file_name, name);
    assert_eq!(view.asset.size, bytes.len());
    let c = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
    let (meta, cipher): (String, Vec<u8>) = c
        .query_row(
            "SELECT metadata,ciphertext FROM file_assets WHERE id=?1",
            [&asset.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!meta.contains(name));
    assert!(!cipher.windows(name.len()).any(|b| b == name.as_bytes()));
    assert!(!cipher.windows(bytes.len()).any(|b| b == bytes));
    drop(c);
    let create = || {
        f.application
            .create_file_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-file-once-0001",
            )
            .unwrap()
    };
    let draft = create();
    assert_eq!(draft.metadata.schema_version, 4);
    assert_eq!(create().metadata.id, draft.metadata.id);
    let preview = f
        .application
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(preview.payload.file.as_ref().unwrap().file_name, name);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    f.approve(&draft);
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn file_deletion_tamper_and_expiry_cancel_before_any_native_attempt() {
    for mode in 0..3 {
        let f = Fixture::new();
        let asset = f
            .application
            .upload_file_local(&f.account, "owned.txt", b"owned bytes".to_vec())
            .unwrap();
        let draft = f
            .application
            .create_file_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-file-failure-0001",
            )
            .unwrap();
        f.approve(&draft);
        if mode == 0 {
            f.application
                .delete_file_local(&f.account, &asset.id)
                .unwrap();
        } else {
            let c = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
            if mode == 1 {
                c.execute(
                    "UPDATE file_assets SET ciphertext=zeroblob(length(ciphertext))",
                    [],
                )
                .unwrap();
            } else {
                c.execute(
                    "UPDATE file_assets SET expires_at='2000-01-01T00:00:00+00:00'",
                    [],
                )
                .unwrap();
            }
            drop(c);
            if mode == 2 {
                assert_eq!(f.repository.purge_expired_files().unwrap(), 1);
            }
        }
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.repository
                .send_status(&f.account, &draft.metadata.id)
                .unwrap()
                .state,
            SendState::Canceled
        );
    }
}

struct Fixture {
    directory: std::path::PathBuf,
    account: String,
    repository: Arc<SqliteRepository>,
    provider: Arc<SyntheticSender>,
    application: Arc<Application>,
    _cleanup: owned_directory::OwnedDirectory,
}
impl Fixture {
    fn new() -> Self {
        let cleanup =
            owned_directory::OwnedDirectory::new("werc-send-fixture");
        let directory=cleanup.0.clone();

        let repository =
            Arc::new(SqliteRepository::open(&directory.join("state.sqlite3")).unwrap());
        let account = repository.register_account("自有发送宿主").unwrap().id;
        let sha = "1".repeat(64);
        repository
            .save_binding(&ClientBinding {
                account_id: account.clone(),
                installation: ClientInstallation {
                    id: "synthetic-only".into(),
                    executable_path: "synthetic-not-executed".into(),
                    version: "synthetic".into(),
                    architecture: "x86_64".into(),
                    product_name: "Owned sender fixture, not WeChat".into(),
                    sha256: sha.clone(),
                    process_ids: vec![],
                    sources: vec![],
                    compatibility: "synthetic".into(),
                    reason: "never calls WeChat".into(),
                },
                bound_at: Utc::now().to_rfc3339(),
                checked_at: Utc::now().to_rfc3339(),
                validation: "synthetic".into(),
            })
            .unwrap();
        let provider = Arc::new(SyntheticSender {
            repository: repository.clone(),
            context: Mutex::new(SendContext {
                sender_id: "synthetic-private-sender-identity".into(),
                target_id: "synthetic-private-recipient-identity".into(),
                target_display_name: "自有模拟接收者".into(),
                identity_verified: true,
                target_verified: true,
                epoch: uuid::Uuid::new_v4().to_string(),
                installation_sha256: sha,
                adapter_sha256: "2".repeat(64),
                execution_mode: SendExecutionMode::Synthetic,
            }),
            mode: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            queries: AtomicUsize::new(0),
            gate: Mutex::new((false, false)),
            wake: Condvar::new(),
        });
        let application = Arc::new(Application::new(repository.clone(), provider.clone()));
        Self {
            directory,
            account,
            repository,
            provider,
            application,
            _cleanup:cleanup,
        }
    }
    fn draft(&self, key: &str) -> SendStatus {
        self.application
            .create_text_send_local(
                &self.account,
                "synthetic-private-recipient-identity",
                "synthetic-private-body-never-sent-to-WeChat",
                key,
            )
            .unwrap()
    }
    fn approve(&self, status: &SendStatus) -> SendStatus {
        self.application
            .approve_text_send_local(
                &self.account,
                &status.metadata.id,
                &status.metadata.request_digest,
                true,
            )
            .unwrap()
    }
    fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.directory.join("state.sqlite3")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.provider.release();
    }
}

#[test]
fn repeated_equal_evidence_refreshes_protected_reference_without_another_submission() {
    let f = Fixture::new();
    let draft = f.draft("synthetic-receipt-refresh-0001");
    f.approve(&draft);
    let status = f
        .application
        .execute_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(status.state, SendState::ServerAccepted);
    let event_count = f
        .database()
        .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
        .unwrap();
    // 存储层将回执视为已由Application校验的opaque保护材料；此处只核更新语义。
    for protected in [vec![1, 2], vec![3, 4]] {
        let receipt = ProtectedReceipt {
            outbox_id: status.metadata.id.clone(),
            attempt_id: status.attempt_id.clone().unwrap(),
            context_digest: status.metadata.context_digest.clone(),
            evidence: SendEvidence::ServerAccepted,
            protected: protected.clone(),
        };
        let updated = f
            .repository
            .record_send_receipt(&f.account, &receipt)
            .unwrap();
        assert_eq!(updated.state, SendState::ServerAccepted);
        assert_eq!(updated.updated_at, status.updated_at);
        assert_eq!(
            f.repository
                .send_receipt(&f.account, &status.metadata.id)
                .unwrap(),
            Some(protected)
        );
    }
    assert_eq!(
        f.database()
            .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        event_count
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.database()
            .query_row("SELECT count(*) FROM send_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.database()
            .query_row("SELECT count(*) FROM send_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn drafts_require_exact_local_approval_and_bind_idempotency_without_plaintext_storage() {
    let f = Fixture::new();
    let a = &f.account;
    let app = &f.application;
    let first = f.draft("synthetic-idempotency-0001");
    assert_eq!(first.state, SendState::AwaitingApproval);
    assert_eq!(
        f.draft("synthetic-idempotency-0001").metadata.id,
        first.metadata.id
    );
    assert_eq!(
        app.create_text_send_local(
            a,
            "synthetic-private-recipient-identity",
            "different content",
            "synthetic-idempotency-0001"
        )
        .unwrap_err()
        .code,
        "IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(
        app.execute_send_local(a, &first.metadata.id)
            .unwrap_err()
            .code,
        "INVALID_TRANSITION"
    );
    assert_eq!(
        app.approve_text_send_local(a, &first.metadata.id, &first.metadata.request_digest, false)
            .unwrap_err()
            .code,
        "CONSENT_REQUIRED"
    );
    assert_eq!(
        app.approve_text_send_local(a, &first.metadata.id, &"f".repeat(64), true)
            .unwrap_err()
            .code,
        "APPROVAL_MISMATCH"
    );
    let view = app.send_preview_local(a, &first.metadata.id).unwrap();
    assert_eq!(
        view.payload.text,
        "synthetic-private-body-never-sent-to-WeChat"
    );
    assert_eq!(f.approve(&first).state, SendState::Queued);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        app.execute_send_local(a, &first.metadata.id).unwrap().state,
        SendState::ServerAccepted
    );
    assert_eq!(
        app.reconcile_send_local(a, &first.metadata.id)
            .unwrap()
            .state,
        SendState::RecipientConfirmed
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.provider.queries.load(Ordering::SeqCst), 1);
    assert_eq!(
        app.execute_send_local(a, &first.metadata.id)
            .unwrap_err()
            .code,
        "INVALID_TRANSITION"
    );
    let bytes = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    for private in [
        "synthetic-private-body-never-sent-to-WeChat",
        "synthetic-private-recipient-identity",
        "synthetic-private-sender-identity",
        "synthetic-private-provider-reference",
    ] {
        assert!(
            !bytes
                .windows(private.len())
                .any(|w| w == private.as_bytes())
        );
    }
    let events = f.repository.events(0, a, 100).unwrap();
    let events = serde_json::to_string(&events).unwrap();
    assert!(!events.contains("synthetic-private"));
}

#[test]
fn lost_ack_local_acceptance_panics_and_mismatched_receipts_never_replay() {
    for mode in 1..=4 {
        let f = Fixture::new();
        f.provider.mode.store(mode, Ordering::SeqCst);
        let draft = f.draft("synthetic-idempotency-0002");
        f.approve(&draft);
        let _ = f
            .application
            .execute_send_local(&f.account, &draft.metadata.id);
        assert_eq!(
            f.repository
                .send_status(&f.account, &draft.metadata.id)
                .unwrap()
                .state,
            SendState::Unknown
        );
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert!(
            f.application
                .cancel_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(
            f.draft("synthetic-idempotency-0002").metadata.id,
            draft.metadata.id
        );
        assert!(!f.application.run_send_once_local().unwrap());
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn crash_recovery_keeps_original_attempt_and_only_queries_receipts() {
    let f = Fixture::new();
    let draft = f.draft("synthetic-idempotency-0003");
    f.approve(&draft);
    let attempt = uuid::Uuid::new_v4().to_string();
    f.repository
        .start_send(
            &f.repository
                .send_status(&f.account, &draft.metadata.id)
                .unwrap(),
            &attempt,
        )
        .unwrap();
    let reopened = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(reopened.clone(), f.provider.clone());
    assert_eq!(app.recover_sends_local().unwrap(), 1);
    assert_eq!(app.recover_sends_local().unwrap(), 0);
    assert_eq!(
        reopened
            .send_status(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert!(!app.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let confirmed = app
        .reconcile_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(confirmed.state, SendState::RecipientConfirmed);
    assert_eq!(confirmed.attempt_id.as_deref(), Some(attempt.as_str()));
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.provider.queries.load(Ordering::SeqCst), 1);
}

#[test]
fn changed_epoch_identity_binding_or_token_revocation_prevents_side_effects() {
    for change in 0..4 {
        let f = Fixture::new();
        let draft = f.draft("synthetic-idempotency-0004");
        f.approve(&draft);
        match change {
            0 => f.provider.context.lock().unwrap().epoch = uuid::Uuid::new_v4().to_string(),
            1 => f.provider.context.lock().unwrap().identity_verified = false,
            2 => f.repository.remove_binding(&f.account).unwrap(),
            _ => f.provider.context.lock().unwrap().adapter_sha256 = "3".repeat(64),
        };
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
    let f = Fixture::new();
    let token = f
        .application
        .issue_token_local(
            "自有发送令牌",
            vec![f.account.clone()],
            vec![
                "messages:send".into(),
                "operations:create".into(),
                "operations:read".into(),
            ],
            1,
        )
        .unwrap();
    f.application.enable_api(true);
    let principal = f.application.authenticate(&token.secret).unwrap();
    let draft = f
        .application
        .create_text_send(
            &principal,
            &f.account,
            "synthetic-private-recipient-identity",
            "synthetic body",
            "synthetic-idempotency-0005",
        )
        .unwrap();
    f.approve(&draft);
    f.application.revoke_token_local(&token.info.id).unwrap();
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "AUTH_REVOKED"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ciphertext_approval_and_metadata_tampering_fail_before_submission() {
    for change in 0..3 {
        let f = Fixture::new();
        let draft = f.draft("synthetic-idempotency-0006");
        f.approve(&draft);
        let raw = f.database();
        match change {
            0 => {
                raw.execute(
                    "UPDATE send_outbox SET ciphertext=zeroblob(length(ciphertext)) WHERE id=?1",
                    [&draft.metadata.id],
                )
                .unwrap();
            }
            1 => {
                raw.execute("UPDATE send_outbox SET approval_proof=zeroblob(length(approval_proof)) WHERE id=?1",[&draft.metadata.id]).unwrap();
            }
            _ => {
                raw.execute("UPDATE send_outbox SET metadata=json_set(metadata,'$.context_digest',?1) WHERE id=?2",rusqlite::params!["f".repeat(64),draft.metadata.id]).unwrap();
            }
        };
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn two_instances_cannot_submit_twice_and_in_flight_cancellation_is_rejected() {
    let f = Fixture::new();
    let draft = f.draft("synthetic-idempotency-0007");
    f.approve(&draft);
    let another = f.draft("synthetic-idempotency-concurrent");
    f.approve(&another);
    f.provider.mode.store(5, Ordering::SeqCst);
    let app = f.application.clone();
    let account = f.account.clone();
    let id = draft.metadata.id.clone();
    let thread = std::thread::spawn(move || app.execute_send_local(&account, &id));
    f.provider.wait_entered();
    let other = Application::new(
        Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap()),
        f.provider.clone(),
    );
    assert!(
        other
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(
        other
            .execute_send_local(&f.account, &another.metadata.id)
            .unwrap_err()
            .code,
        "BUSY"
    );
    assert!(
        other
            .cancel_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    f.provider.release();
    assert_eq!(
        thread.join().unwrap().unwrap().state,
        SendState::ServerAccepted
    );
}

#[test]
fn expired_approval_is_not_submitted_and_recovery_never_requeues_unknown() {
    let f = Fixture::new();
    let draft = f.draft("synthetic-idempotency-0008");
    let until = Utc::now().timestamp() + 1;
    let proof=serde_json::to_vec(&serde_json::json!({"id":draft.metadata.id,"request_digest":draft.metadata.request_digest,"context_digest":draft.metadata.context_digest,"expires_at":until,"approver":"desktop"})).unwrap();
    let proof = f
        .provider
        .protect_key(
            &format!(
                "WeRC/outbox-approval/v1/{}/{}",
                f.account, draft.metadata.id
            ),
            &proof,
        )
        .unwrap();
    f.repository
        .approve_send(
            &f.account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            &draft.metadata.context_digest,
            until,
            &proof,
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "APPROVAL_EXPIRED"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.repository
            .send_status(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Expired
    );
}

#[tokio::test]
async fn api_drafts_never_self_approve_and_cancellation_is_creator_scoped() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let issue = |label: &str| {
        f.application
            .issue_token_local(
                label,
                vec![f.account.clone()],
                vec![
                    "messages:send".into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let first = issue("自有API一");
    let second = issue("自有API二");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, method: &str, uri: &str, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "synthetic-api-key-0001")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let input = serde_json::json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","text":"synthetic-private-body","wait_seconds":0});
    let response = router
        .clone()
        .oneshot(request(
            &first.secret,
            "POST",
            "/api/v1/messages/send-text",
            input.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["state"], "awaiting_approval");
    assert!(!body.to_string().contains("synthetic-private"));
    let id = body["data"]["metadata"]["id"].as_str().unwrap();
    let response = router
        .clone()
        .oneshot(request(
            &first.secret,
            "POST",
            "/api/v1/messages/send-text",
            input,
        ))
        .await
        .unwrap();
    let same: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(same["data"]["metadata"]["id"], id);
    let uri = format!("/api/v1/outbox/{id}?account_id={}", f.account);
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &second.secret,
                "DELETE",
                &uri,
                serde_json::Value::Null
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(&first.secret, "GET", &uri, serde_json::Value::Null))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &first.secret,
                "POST",
                &format!("/api/v1/outbox/{id}/approve"),
                serde_json::json!({"confirmed":true})
            ))
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &first.secret,
                "DELETE",
                &uri,
                serde_json::Value::Null
            ))
            .await
            .unwrap()
            .status(),
        200
    );
    let reader = f
        .application
        .issue_token_local(
            "只读",
            vec![f.account.clone()],
            vec!["operations:read".into()],
            1,
        )
        .unwrap();
    assert_eq!(router.clone().oneshot(request(&reader.secret,"POST","/api/v1/messages/send-text",serde_json::json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","text":"body"}))).await.unwrap().status(),403);
    f.application.revoke_token_local(&first.info.id).unwrap();
    assert_eq!(
        router
            .oneshot(request(&first.secret, "GET", &uri, serde_json::Value::Null))
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn final_transaction_rejects_changed_approval_and_missing_secret_never_regenerates() {
    let f = Fixture::new();
    let draft = f.draft("synthetic-idempotency-0010");
    let approved = f.approve(&draft);
    f.database()
        .execute(
            "UPDATE send_outbox SET approval_expires_at=approval_expires_at+1 WHERE id=?1",
            [&draft.metadata.id],
        )
        .unwrap();
    assert_eq!(
        f.repository
            .start_send(&approved, &uuid::Uuid::new_v4().to_string())
            .unwrap_err()
            .code,
        "CONNECTION_CHANGED"
    );
    assert_eq!(
        f.database()
            .query_row("SELECT count(*) FROM send_attempts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.database()
        .execute("DELETE FROM send_secrets", [])
        .unwrap();
    assert_eq!(
        f.application
            .create_text_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                "body",
                "synthetic-idempotency-0011"
            )
            .unwrap_err()
            .code,
        "OUTBOX_CORRUPTED"
    );
    assert!(
        f.repository
            .send_secret(&f.account, None)
            .unwrap()
            .is_none()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn unknown_receipts_never_cross_identity_epochs_and_evidence_cannot_downgrade() {
    let f = Fixture::new();
    f.provider.mode.store(1, Ordering::SeqCst);
    let draft = f.draft("synthetic-idempotency-0012");
    f.approve(&draft);
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    f.provider.context.lock().unwrap().epoch = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        f.application
            .reconcile_send_local(&f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "CONNECTION_CHANGED"
    );
    assert_eq!(f.provider.queries.load(Ordering::SeqCst), 0);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    for state in [SendState::ServerAccepted, SendState::RecipientConfirmed] {
        for evidence in [
            SendEvidence::LocalAccepted,
            SendEvidence::NativeQueued,
            SendEvidence::Unknown,
            SendEvidence::RejectedBeforeSubmission,
        ] {
            assert!(state.apply_evidence(evidence).is_err());
        }
    }
    for state in [SendState::Canceled, SendState::Expired, SendState::Rejected] {
        assert!(state.apply_evidence(SendEvidence::ServerAccepted).is_err());
    }
}

#[test]
fn queue_quota_and_payload_limits_do_not_evict_unknown_or_history() {
    let f = Fixture::new();
    assert!(
        f.application
            .create_text_send_local(
                &f.account,
                "table:synthetic",
                "body",
                "synthetic-idempotency-0013"
            )
            .is_err()
    );
    assert!(
        f.application
            .create_text_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &"x".repeat(16385),
                "synthetic-idempotency-0013"
            )
            .is_err()
    );
    let mut ids = std::collections::BTreeSet::new();
    for i in 0..512 {
        ids.insert(f.draft(&format!("synthetic-quota-key-{i:04}")).metadata.id);
    }
    assert_eq!(
        f.application
            .create_text_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                "body",
                "synthetic-quota-overflow"
            )
            .unwrap_err()
            .code,
        "QUOTA_EXCEEDED"
    );
    assert_eq!(
        f.database()
            .query_row("SELECT count(*) FROM send_outbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        512
    );
    let listed = f.application.sends_local(&f.account).unwrap();
    assert_eq!(listed.len(), 512, "retained tasks must remain discoverable");
    assert_eq!(
        listed
            .into_iter()
            .map(|s| s.metadata.id)
            .collect::<std::collections::BTreeSet<_>>(),
        ids
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn unverified_and_unavailable_providers_cannot_create_ready_tasks_or_silently_use_uia() {
    let f = Fixture::new();
    f.provider.context.lock().unwrap().identity_verified = false;
    assert_eq!(
        f.application
            .create_text_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                "body",
                "synthetic-idempotency-0009"
            )
            .unwrap_err()
            .code,
        "IDENTITY_UNVERIFIED"
    );
    assert!(f.repository.sends(&f.account).unwrap().is_empty());
    let production = Application::new(f.repository.clone(), Arc::new(WindowsWechatProvider));
    assert_eq!(
        production
            .create_text_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                "body",
                "synthetic-idempotency-0009"
            )
            .unwrap_err()
            .code,
        "CAPABILITY_NOT_IMPLEMENTED"
    );
    assert!(f.repository.sends(&f.account).unwrap().is_empty());
}
#[tokio::test]
async fn video_api_requires_its_own_scope_and_never_approves_or_sends() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let asset = f
        .application
        .upload_video_local(
            &f.account,
            "自动化测试.mp4",
            include_bytes!("../../../tests/fixtures/owned-send-video.mp4").to_vec(),
        )
        .unwrap();
    let issue = |scope: &str| {
        f.application
            .issue_token_local(
                "自有文件API",
                vec![f.account.clone()],
                vec![
                    scope.into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let text = issue("messages:send");
    let file = issue("messages:send_video");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, method: &str, uri: &str, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "owned-video-api-0001")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let input = serde_json::json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","asset_id":asset.id});
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &text.secret,
                "POST",
                "/api/v1/messages/send-video",
                input.clone()
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router
        .clone()
        .oneshot(request(
            &file.secret,
            "POST",
            "/api/v1/messages/send-video",
            input,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["state"], "awaiting_approval");
    assert!(!body.to_string().contains(&asset.id));
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let id = body["data"]["metadata"]["id"].as_str().unwrap();
    let uri = format!("/api/v1/outbox/{id}?account_id={}", f.account);
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &file.secret,
                "DELETE",
                &uri,
                serde_json::Value::Null
            ))
            .await
            .unwrap()
            .status(),
        200
    );
    let principal = f.application.authenticate(&file.secret).unwrap();
    let draft = f
        .application
        .create_video_send(
            &principal,
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            "owned-video-api-0002",
        )
        .unwrap();
    f.approve(&draft);
    f.application.revoke_token_local(&file.info.id).unwrap();
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn video_names_and_bytes_are_encrypted_and_unknown_is_never_replayed() {
    let f = Fixture::new();
    let bytes = include_bytes!("../../../tests/fixtures/owned-send-video.mp4");
    let name = "私有自有视频.mp4";
    let asset = f
        .application
        .upload_video_local(&f.account, name, bytes.to_vec())
        .unwrap();
    let view = f
        .application
        .file_preview_local(&f.account, &asset.id)
        .unwrap();
    assert_eq!(view.file_name, name);
    assert_eq!(view.asset.size, bytes.len());
    let c = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
    let (meta, cipher): (String, Vec<u8>) = c
        .query_row(
            "SELECT metadata,ciphertext FROM file_assets WHERE id=?1",
            [&asset.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!meta.contains(name));
    assert!(!cipher.windows(name.len()).any(|b| b == name.as_bytes()));
    assert!(!cipher.windows(bytes.len()).any(|b| b == bytes));
    drop(c);
    let create = || {
        f.application
            .create_video_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-video-once-0001",
            )
            .unwrap()
    };
    let draft = create();
    assert_eq!(draft.metadata.schema_version, 5);
    assert_eq!(create().metadata.id, draft.metadata.id);
    let preview = f
        .application
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(preview.payload.video.as_ref().unwrap().file.file_name, name);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    f.approve(&draft);
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn video_deletion_tamper_and_expiry_cancel_before_any_native_attempt() {
    for mode in 0..3 {
        let f = Fixture::new();
        let asset = f
            .application
            .upload_video_local(
                &f.account,
                "owned.mp4",
                include_bytes!("../../../tests/fixtures/owned-send-video.mp4").to_vec(),
            )
            .unwrap();
        let draft = f
            .application
            .create_video_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-video-failure-0001",
            )
            .unwrap();
        f.approve(&draft);
        if mode == 0 {
            f.application
                .delete_file_local(&f.account, &asset.id)
                .unwrap();
        } else {
            let c = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
            if mode == 1 {
                c.execute(
                    "UPDATE file_assets SET ciphertext=zeroblob(length(ciphertext))",
                    [],
                )
                .unwrap();
            } else {
                c.execute(
                    "UPDATE file_assets SET expires_at='2000-01-01T00:00:00+00:00'",
                    [],
                )
                .unwrap();
            }
            drop(c);
            if mode == 2 {
                assert_eq!(f.repository.purge_expired_files().unwrap(), 1);
            }
        }
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.repository
                .send_status(&f.account, &draft.metadata.id)
                .unwrap()
                .state,
            SendState::Canceled
        );
    }
}

#[test]
fn video_rejects_file_disguises_duration_and_changed_idempotency_before_submission() {
    let f = Fixture::new();
    let bytes = include_bytes!("../../../tests/fixtures/owned-send-video.mp4");
    assert!(
        f.application
            .upload_video_local(&f.account, "owned.txt", bytes.to_vec())
            .is_err()
    );
    assert!(
        f.application
            .upload_video_local(&f.account, "owned.mp4", b"not an mp4".to_vec())
            .is_err()
    );
    assert!(
        f.application
            .upload_video_local(
                &f.account,
                "short.mp4",
                include_bytes!("../../../tests/fixtures/owned-video.mp4").to_vec()
            )
            .is_err()
    );
    let asset = f
        .application
        .upload_video_local(&f.account, "owned.mp4", bytes.to_vec())
        .unwrap();
    let key = "owned-video-conflict-0001";
    let draft = f
        .application
        .create_file_send_local(
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            key,
        )
        .unwrap();
    assert!(
        f.application
            .create_video_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                key
            )
            .is_err()
    );
    assert_eq!(f.repository.sends(&f.account).unwrap().len(), 1);
    assert_eq!(draft.metadata.schema_version, 4);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn emoticon_api_requires_its_own_scope_and_never_approves_or_sends() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let asset = f
        .application
        .upload_emoticon_local(
            &f.account,
            "自动化测试.gif",
            include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif").to_vec(),
        )
        .unwrap();
    let issue = |scope: &str| {
        f.application
            .issue_token_local(
                "自有文件API",
                vec![f.account.clone()],
                vec![
                    scope.into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let text = issue("messages:send");
    let file = issue("messages:send_emoticon");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, method: &str, uri: &str, body: serde_json::Value| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "owned-emoticon-api-0001")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let input = serde_json::json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","asset_id":asset.id});
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &text.secret,
                "POST",
                "/api/v1/messages/send-emoticon",
                input.clone()
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router
        .clone()
        .oneshot(request(
            &file.secret,
            "POST",
            "/api/v1/messages/send-emoticon",
            input,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"]["state"], "awaiting_approval");
    assert!(!body.to_string().contains(&asset.id));
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let id = body["data"]["metadata"]["id"].as_str().unwrap();
    let uri = format!("/api/v1/outbox/{id}?account_id={}", f.account);
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                &file.secret,
                "DELETE",
                &uri,
                serde_json::Value::Null
            ))
            .await
            .unwrap()
            .status(),
        200
    );
    let principal = f.application.authenticate(&file.secret).unwrap();
    let draft = f
        .application
        .create_emoticon_send(
            &principal,
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            "owned-emoticon-api-0002",
        )
        .unwrap();
    f.approve(&draft);
    f.application.revoke_token_local(&file.info.id).unwrap();
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn emoticon_names_and_bytes_are_encrypted_and_unknown_is_never_replayed() {
    let f = Fixture::new();
    let bytes = include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif");
    let name = "私有自有表情.gif";
    let asset = f
        .application
        .upload_emoticon_local(&f.account, name, bytes.to_vec())
        .unwrap();
    let view = f
        .application
        .file_preview_local(&f.account, &asset.id)
        .unwrap();
    assert_eq!(view.file_name, name);
    assert_eq!(view.asset.size, bytes.len());
    let c = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
    let (meta, cipher): (String, Vec<u8>) = c
        .query_row(
            "SELECT metadata,ciphertext FROM file_assets WHERE id=?1",
            [&asset.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!meta.contains(name));
    assert!(!cipher.windows(name.len()).any(|b| b == name.as_bytes()));
    assert!(!cipher.windows(bytes.len()).any(|b| b == bytes));
    drop(c);
    let create = || {
        f.application
            .create_emoticon_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-emoticon-once-0001",
            )
            .unwrap()
    };
    let draft = create();
    assert_eq!(draft.metadata.schema_version, 6);
    assert_eq!(create().metadata.id, draft.metadata.id);
    let preview = f
        .application
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(
        preview.payload.emoticon.as_ref().unwrap().file.file_name,
        name
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    f.approve(&draft);
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn emoticon_deletion_tamper_and_expiry_cancel_before_any_native_attempt() {
    for mode in 0..3 {
        let f = Fixture::new();
        let asset = f
            .application
            .upload_emoticon_local(
                &f.account,
                "owned.gif",
                include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif").to_vec(),
            )
            .unwrap();
        let draft = f
            .application
            .create_emoticon_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                "owned-emoticon-failure-0001",
            )
            .unwrap();
        f.approve(&draft);
        if mode == 0 {
            f.application
                .delete_file_local(&f.account, &asset.id)
                .unwrap();
        } else {
            let c = rusqlite::Connection::open(f.directory.join("state.sqlite3")).unwrap();
            if mode == 1 {
                c.execute(
                    "UPDATE file_assets SET ciphertext=zeroblob(length(ciphertext))",
                    [],
                )
                .unwrap();
            } else {
                c.execute(
                    "UPDATE file_assets SET expires_at='2000-01-01T00:00:00+00:00'",
                    [],
                )
                .unwrap();
            }
            drop(c);
            if mode == 2 {
                assert_eq!(f.repository.purge_expired_files().unwrap(), 1);
            }
        }
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.repository
                .send_status(&f.account, &draft.metadata.id)
                .unwrap()
                .state,
            SendState::Canceled
        );
    }
}

#[test]
fn emoticon_rejects_file_disguises_duration_and_changed_idempotency_before_submission() {
    let f = Fixture::new();
    let bytes = include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif");
    assert!(
        f.application
            .upload_emoticon_local(&f.account, "owned.txt", bytes.to_vec())
            .is_err()
    );
    assert!(
        f.application
            .upload_emoticon_local(&f.account, "owned.gif", b"not an mp4".to_vec())
            .is_err()
    );
    assert!(
        f.application
            .upload_emoticon_local(&f.account, "short.gif", b"GIF89a".to_vec())
            .is_err()
    );
    let asset = f
        .application
        .upload_emoticon_local(&f.account, "owned.gif", bytes.to_vec())
        .unwrap();
    let key = "owned-emoticon-conflict-0001";
    let draft = f
        .application
        .create_file_send_local(
            &f.account,
            "synthetic-private-recipient-identity",
            &asset.id,
            key,
        )
        .unwrap();
    assert!(
        f.application
            .create_emoticon_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &asset.id,
                key
            )
            .is_err()
    );
    assert_eq!(f.repository.sends(&f.account).unwrap().len(), 1);
    assert_eq!(draft.metadata.schema_version, 4);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn utf8_ten_thousand_byte_draft_is_exact_encrypted_and_excess_is_refused() {
    let f = Fixture::new();
    let body = format!("{}x", "中".repeat(3333));
    assert_eq!(body.len(), 10000);
    let draft = f
        .application
        .create_text_send_local(
            &f.account,
            "synthetic-private-recipient-identity",
            &body,
            "owned-long-text-once-0001",
        )
        .unwrap();
    assert_eq!(draft.metadata.schema_version, 1);
    let preview = f
        .application
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(preview.payload.text, body);
    let bytes = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(!bytes.windows(64).any(|w| w == &body.as_bytes()[..64]));
    for invalid in [format!("{body}x"), "中".repeat(3334), "x\0y".into()] {
        let e = f
            .application
            .create_text_send_local(
                &f.account,
                "synthetic-private-recipient-identity",
                &invalid,
                "owned-long-invalid-0001",
            )
            .unwrap_err();
        assert_eq!(e.code, "INVALID_ARGUMENT");
    }
    assert_eq!(f.application.sends_local(&f.account).unwrap().len(), 1);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    f.approve(&draft);
    f.application
        .execute_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}

mod send_wait {
    use super::*;
    use axum::{body::Body, http::Request, response::Response};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::time::Duration;
    use tower::ServiceExt;

    fn token(f: &Fixture) -> werc_domain::IssuedToken {
        f.application
            .issue_token_local(
                "自动化测试等待",
                vec![f.account.clone()],
                vec!["messages:send".into(), "operations:create".into()],
                1,
            )
            .unwrap()
    }
    fn request(f: &Fixture, secret: &str, key: &str, wait: Option<Value>) -> Request<Body> {
        let mut body = json!({"account_id":f.account,"username":"synthetic-private-recipient-identity","text":"自动化测试：只对自有提供方等待"});
        if let Some(wait) = wait {
            body["wait_seconds"] = wait;
        }
        Request::builder()
            .method("POST")
            .uri("/api/v1/messages/send-text")
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", key)
            .body(Body::from(body.to_string()))
            .unwrap()
    }
    async fn body(response: Response) -> Value {
        assert_eq!(response.status(), 200);
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
    }
    async fn drafts(f: &Fixture, count: usize) -> Vec<SendStatus> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let rows = f.application.sends_local(&f.account).unwrap();
                if rows.len() == count {
                    return rows;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn zero_timeout_default_and_idempotency_preserve_unapproved_draft() {
        let f = Fixture::new();
        let token = token(&f);
        f.application.enable_api(true);
        let router = werc_http_api::router(f.application.clone());
        let key = "synthetic-send-wait-0001";
        let first = body(
            tokio::time::timeout(
                Duration::from_secs(3),
                router
                    .clone()
                    .oneshot(request(&f, &token.secret, key, Some(json!(0)))),
            )
            .await
            .unwrap()
            .unwrap(),
        )
        .await;
        assert_eq!(first["data"]["state"], "awaiting_approval");
        let started = std::time::Instant::now();
        let waited = body(
            router
                .clone()
                .oneshot(request(&f, &token.secret, key, Some(json!(1))))
                .await
                .unwrap(),
        )
        .await;
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert_eq!(
            first["data"]["metadata"]["id"],
            waited["data"]["metadata"]["id"]
        );
        assert_eq!(waited["data"]["state"], "awaiting_approval");
        let pending = tokio::spawn(router.oneshot(request(&f, &token.secret, key, None)));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!pending.is_finished());
        let id = first["data"]["metadata"]["id"].as_str().unwrap();
        f.application.cancel_send_local(&f.account, id).unwrap();
        let canceled = body(
            tokio::time::timeout(Duration::from_secs(3), pending)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        )
        .await;
        assert_eq!(canceled["data"]["state"], "canceled");
        assert_eq!(f.application.sends_local(&f.account).unwrap().len(), 1);
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn local_approval_and_submission_end_wait_without_repeating_unknown() {
        for (mode, expected) in [(0, "server_accepted"), (1, "unknown")] {
            let f = Fixture::new();
            let token = token(&f);
            f.application.enable_api(true);
            f.provider.mode.store(mode, Ordering::SeqCst);
            let router = werc_http_api::router(f.application.clone());
            let key = "synthetic-send-wait-0002";
            let pending = tokio::spawn(router.clone().oneshot(request(
                &f,
                &token.secret,
                key,
                Some(json!(120)),
            )));
            let draft = drafts(&f, 1).await.remove(0);
            assert_eq!(draft.state, SendState::AwaitingApproval);
            assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
            f.approve(&draft);
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .unwrap();
            let response = body(
                tokio::time::timeout(Duration::from_secs(3), pending)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap(),
            )
            .await;
            assert_eq!(response["data"]["state"], expected);
            let repeat = body(
                tokio::time::timeout(
                    Duration::from_secs(3),
                    router.oneshot(request(&f, &token.secret, key, Some(json!(120)))),
                )
                .await
                .unwrap()
                .unwrap(),
            )
            .await;
            assert_eq!(repeat["data"]["metadata"]["id"], draft.metadata.id);
            assert_eq!(repeat["data"]["attempt_id"], response["data"]["attempt_id"]);
            assert_eq!(repeat["data"]["state"], expected);
            assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
            assert_eq!(f.provider.queries.load(Ordering::SeqCst), 0);
        }
    }
    #[tokio::test]
    async fn revocation_and_api_pause_interrupt_wait_without_status_disclosure() {
        for pause in [false, true] {
            let f = Fixture::new();
            let token = token(&f);
            f.application.enable_api(true);
            let router = werc_http_api::router(f.application.clone());
            let pending = tokio::spawn(router.oneshot(request(
                &f,
                &token.secret,
                "synthetic-send-wait-0003",
                Some(json!(120)),
            )));
            drafts(&f, 1).await;
            if pause {
                f.application.enable_api(false);
            } else {
                f.application.revoke_token_local(&token.info.id).unwrap();
            }
            let response = tokio::time::timeout(Duration::from_secs(3), pending)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(response.status().as_u16(), if pause { 503 } else { 401 });
            let value: Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert!(value.get("data").is_none());
            assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        }
    }
    #[tokio::test]
    async fn invalid_wait_is_rejected_before_persistence() {
        let f = Fixture::new();
        let token = token(&f);
        f.application.enable_api(true);
        let router = werc_http_api::router(f.application.clone());
        for invalid in [json!(121), json!(-1), json!(0.5), json!("30"), json!(null)] {
            let response = router
                .clone()
                .oneshot(request(
                    &f,
                    &token.secret,
                    "synthetic-send-wait-0004",
                    Some(invalid),
                ))
                .await
                .unwrap();
            assert!(matches!(response.status().as_u16(), 400 | 422));
        }
        assert!(f.application.sends_local(&f.account).unwrap().is_empty());
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn wait_admission_and_request_cancel_do_not_cancel_or_send_drafts() {
        let f = Fixture::new();
        let token = token(&f);
        f.application.enable_api(true);
        let router = werc_http_api::router(f.application.clone());
        let mut pending = Vec::new();
        for i in 0..16 {
            pending.push(tokio::spawn(router.clone().oneshot(request(
                &f,
                &token.secret,
                &format!("synthetic-send-wait-cap-{i:02}"),
                Some(json!(120)),
            ))));
        }
        drafts(&f, 16).await;
        let rejected = router
            .clone()
            .oneshot(request(
                &f,
                &token.secret,
                "synthetic-send-wait-cap-extra",
                Some(json!(1)),
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), 429);
        assert_eq!(f.application.sends_local(&f.account).unwrap().len(), 16);
        let immediate = body(
            router
                .clone()
                .oneshot(request(
                    &f,
                    &token.secret,
                    "synthetic-send-wait-cap-zero",
                    Some(json!(0)),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(immediate["data"]["state"], "awaiting_approval");
        for task in &pending {
            task.abort();
        }
        for task in pending {
            assert!(task.await.unwrap_err().is_cancelled());
        }
        let rows = f.application.sends_local(&f.account).unwrap();
        assert_eq!(rows.len(), 17);
        assert!(
            rows.iter()
                .all(|r| r.state == SendState::AwaitingApproval && r.attempt_id.is_none())
        );
        let next = tokio::spawn(router.oneshot(request(
            &f,
            &token.secret,
            "synthetic-send-wait-cap-next",
            Some(json!(120)),
        )));
        let rows = drafts(&f, 18).await;
        for row in rows {
            f.application
                .cancel_send_local(&f.account, &row.metadata.id)
                .unwrap();
        }
        assert_eq!(
            body(
                tokio::time::timeout(Duration::from_secs(3), next)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
            )
            .await["data"]["state"],
            "canceled"
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn status_for_creator_does_not_grant_read_access_to_other_tasks() {
        let f = Fixture::new();
        let first = token(&f);
        let second = token(&f);
        f.application.enable_api(true);
        let p = f.application.authenticate(&first.secret).unwrap();
        let other = f.application.authenticate(&second.secret).unwrap();
        let draft = f
            .application
            .create_text_send(
                &p,
                &f.account,
                "synthetic-private-recipient-identity",
                "自动化测试",
                "synthetic-send-wait-0005",
            )
            .unwrap();
        assert!(
            f.application
                .created_text_send_status(&p, &f.account, &draft.metadata.id)
                .is_ok()
        );
        assert_eq!(
            f.application
                .created_text_send_status(&other, &f.account, &draft.metadata.id)
                .unwrap_err()
                .code,
            "FORBIDDEN"
        );
        assert_eq!(
            f.application
                .send_status(&p, &f.account, &draft.metadata.id)
                .unwrap_err()
                .code,
            "FORBIDDEN"
        );
    }
}

#[test]
fn private_moment_encrypted_reopen_approval_and_unknown_never_resubmit() {
    for fail in [false, true] {
        let f = Fixture::new();
        {
            let mut c = f.provider.context.lock().unwrap();
            c.target_id = c.sender_id.clone();
        }
        let body = "自动化测试 私密动态 ".repeat(20);
        let draft = f
            .application
            .create_private_moment_local(&f.account, &body, "private-moment-owned-0001")
            .unwrap();
        assert_eq!(draft.metadata.schema_version, 9);
        assert_eq!(draft.state, SendState::AwaitingApproval);
        assert_eq!(
            f.application
                .create_private_moment_local(&f.account, &body, "private-moment-owned-0001")
                .unwrap()
                .metadata
                .id,
            draft.metadata.id
        );
        assert!(
            f.application
                .create_private_moment_local(&f.account, "changed", "private-moment-owned-0001")
                .is_err()
        );
        let protected = f
            .repository
            .send_payload(&f.account, &draft.metadata.id)
            .unwrap();
        assert!(
            !protected
                .ciphertext
                .windows(body.len())
                .any(|w| w == body.as_bytes())
        );
        let reopened =
            Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
        let app = Application::new(reopened, f.provider.clone());
        let preview = app
            .send_preview_local(&f.account, &draft.metadata.id)
            .unwrap();
        assert_eq!(preview.payload.text, body);
        assert_eq!(
            preview.payload.moment.as_ref().unwrap().visibility,
            MomentVisibility::Private
        );
        assert!(
            app.execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
        f.approve(&draft);
        if fail {
            f.provider.mode.store(2, Ordering::SeqCst);
        }
        let status = app
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap();
        assert_eq!(
            status.state,
            if fail {
                SendState::Unknown
            } else {
                SendState::ServerAccepted
            }
        );
        assert!(!app.run_send_once_local().unwrap());
        assert!(
            app.execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(
            app.reconcile_send_local(&f.account, &draft.metadata.id)
                .unwrap()
                .state,
            SendState::ServerAccepted
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn private_moment_identity_changes_cancel_and_text_limits_prevent_creation() {
    let f = Fixture::new();
    {
        let mut c = f.provider.context.lock().unwrap();
        c.target_id = c.sender_id.clone();
    }
    for text in [" ".into(), "x".repeat(10001), "a\0b".into()] {
        assert!(
            f.application
                .create_private_moment_local(&f.account, &text, "moment-invalid-owned")
                .is_err()
        );
    }
    let draft = f
        .application
        .create_private_moment_local(
            &f.account,
            "自动化测试 取消动态",
            "moment-cancel-owned-0001",
        )
        .unwrap();
    f.approve(&draft);
    f.provider.context.lock().unwrap().epoch = "changed".into();
    assert!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(
        f.repository
            .send_status(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Canceled
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn private_moment_api_has_own_scope_cannot_approve_and_rechecks_revocation() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    {
        let mut c = f.provider.context.lock().unwrap();
        c.target_id = c.sender_id.clone();
    }
    let issue = |scope: &str| {
        f.application
            .issue_token_local(
                "自有动态API",
                vec![f.account.clone()],
                vec![
                    scope.into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let text = issue("messages:send");
    let moment = issue("moments:publish");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, visibility: &str| {
        Request::builder().method("POST").uri("/api/v1/moments/publish").header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json").header("Idempotency-Key","owned-moment-api-0001").body(Body::from(serde_json::json!({"account_id":f.account,"text":"自动化测试 API私密动态","visibility":visibility}).to_string())).unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request(&text.secret, "private"))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(&moment.secret, "friends_except"))
            .await
            .unwrap()
            .status(),
        422
    );
    let response = router
        .oneshot(request(&moment.secret, "private"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["data"]["state"], "awaiting_approval");
    assert_eq!(value["data"]["metadata"]["schema_version"], 9);
    assert!(!value.to_string().contains("API私密动态"));
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let id = value["data"]["metadata"]["id"].as_str().unwrap();
    let draft = f.repository.send_status(&f.account, id).unwrap();
    let principal = f.application.authenticate(&text.secret).unwrap();
    assert!(
        f.application
            .cancel_send(&principal, &f.account, id)
            .is_err()
    );
    f.approve(&draft);
    f.application.revoke_token_local(&moment.info.id).unwrap();
    assert!(f.application.execute_send_local(&f.account, id).is_err());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn planned_send_survives_reopen_never_attempts_early_and_binds_approved_time() {
    let f = Fixture::new();
    f.provider.context.lock().unwrap().sender_id = "synthetic-private-recipient-identity".into();
    let draft = f.draft("owned-planned-once-0001");
    let at = Utc::now().timestamp() + 120;
    let scheduled = f
        .application
        .approve_timed_send_local(
            &f.account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
            Some(at),
        )
        .unwrap();
    assert_eq!(scheduled.scheduled_at, Some(at));
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "SEND_NOT_DUE"
    );
    let reopened = SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap();
    let saved = reopened
        .send_status(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(saved.state, SendState::Queued);
    assert_eq!(saved.scheduled_at, Some(at));
    assert!(saved.attempt_id.is_none());
    // Altering only the persisted schedule cannot move the authenticated approval.
    f.database()
        .execute(
            "UPDATE send_schedules SET scheduled_at=?1 WHERE outbox_id=?2",
            rusqlite::params![Utc::now().timestamp() - 1, draft.metadata.id],
        )
        .unwrap();
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "OUTBOX_CORRUPTED"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn future_plan_does_not_block_immediate_queue_and_can_be_canceled() {
    let f = Fixture::new();
    f.provider.context.lock().unwrap().sender_id = "synthetic-private-recipient-identity".into();
    let planned = f.draft("owned-planned-queue-0001");
    f.application
        .approve_timed_send_local(
            &f.account,
            &planned.metadata.id,
            &planned.metadata.request_digest,
            true,
            Some(Utc::now().timestamp() + 120),
        )
        .unwrap();
    let immediate = f.draft("owned-immediate-queue-0001");
    f.approve(&immediate);
    assert!(f.application.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    let canceled = f
        .application
        .cancel_send_local(&f.account, &planned.metadata.id)
        .unwrap();
    assert_eq!(canceled.state, SendState::Canceled);
    assert!(canceled.attempt_id.is_none());
    assert!(!f.application.run_send_once_local().unwrap());
}

#[test]
fn due_plan_submits_once_and_never_repeats_after_worker_restart() {
    let f = Fixture::new();
    f.provider.context.lock().unwrap().sender_id = "synthetic-private-recipient-identity".into();
    let draft = f.draft("owned-due-plan-0001");
    let at = Utc::now().timestamp() + 2;
    f.application
        .approve_timed_send_local(
            &f.account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
            Some(at),
        )
        .unwrap();
    assert!(!f.application.run_send_once_local().unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while Utc::now().timestamp() < at {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let repo = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let restarted = Application::new(repo, f.provider.clone());
    assert!(restarted.run_send_once_local().unwrap());
    assert!(!restarted.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    assert!(
        f.repository
            .send_status(&f.account, &draft.metadata.id)
            .unwrap()
            .attempt_id
            .is_some()
    );
}

#[test]
fn public_moment_binds_visibility_reopens_and_bot_retry_never_requeues() {
    for with_images in [false, true] {
        let f = Fixture::new();
        {
            let mut c = f.provider.context.lock().unwrap();
            c.target_id = c.sender_id.clone();
        }
        let images = if with_images {
            vec![
                f.application
                    .upload_image_local(
                        &f.account,
                        include_bytes!("../../../tests/fixtures/owned-image.png").to_vec(),
                    )
                    .unwrap()
                    .id,
            ]
        } else {
            vec![]
        };
        let token = f
            .application
            .issue_token_local(
                "测试朋友圈Bot",
                vec![f.account.clone()],
                vec![
                    "moments:publish".into(),
                    "operations:create".into(),
                    "messages:execute".into(),
                ],
                1,
            )
            .unwrap();
        f.application.enable_api(true);
        let p = f.application.authenticate(&token.secret).unwrap();
        let text = "自动化测试：公开朋友圈";
        let key = "owned-public-moment-bot-01";
        let created = f
            .application
            .create_moment(&p, &f.account, text, &images, MomentVisibility::Public, key)
            .unwrap();
        assert_eq!(created.state, SendState::Queued);
        let repo = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
        let app = Application::new(repo, f.provider.clone());
        app.enable_api(true);
        let preview = app
            .send_preview_local(&f.account, &created.metadata.id)
            .unwrap();
        assert_eq!(
            preview.payload.moment.as_ref().unwrap().visibility,
            MomentVisibility::Public
        );
        assert_eq!(
            app.create_moment(
                &p,
                &f.account,
                text,
                &images,
                MomentVisibility::Private,
                key
            )
            .unwrap_err()
            .code,
            "IDEMPOTENCY_CONFLICT"
        );
        app.execute_send_local(&f.account, &created.metadata.id)
            .unwrap();
        let saved = f
            .repository
            .send_status(&f.account, &created.metadata.id)
            .unwrap();
        f.provider.context.lock().unwrap().epoch = "reconnected".into();
        for id in &images {
            app.delete_image_local(&f.account, id).unwrap();
        }
        let retried = app
            .create_moment(&p, &f.account, text, &images, MomentVisibility::Public, key)
            .unwrap();
        assert_eq!(retried.state, saved.state);
        assert_eq!(retried.attempt_id, saved.attempt_id);
        assert!(!app.run_send_once_local().unwrap());
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn moment_action_encrypted_reopen_binds_exact_id_and_never_resubmits() {
    for action in [
        MomentActionKind::Like,
        MomentActionKind::Comment,
        MomentActionKind::Unlike,
        MomentActionKind::Delete,
        MomentActionKind::Reply,
    ] {
        let f = Fixture::new();
        {
            let mut c = f.provider.context.lock().unwrap();
            c.target_id = c.sender_id.clone();
        }
        let request = MomentAction {
            reply: (action == MomentActionKind::Reply).then(|| MomentReplyTarget {
                username: "owned-user".into(),
                id: Some("42".into()),
                id64: Some(u64::MAX.to_string()),
            }),
            sns_id: u64::MAX.to_string(),
            action,
        };
        let text = if matches!(action, MomentActionKind::Comment | MomentActionKind::Reply) {
            "自动化测试：评论"
        } else {
            ""
        };
        let key = "owned-moment-interaction-0001";
        let draft = f
            .application
            .create_moment_action_local(&f.account, &request, text, key)
            .unwrap();
        assert_eq!(draft.metadata.schema_version, 11);
        assert_eq!(
            f.application
                .create_moment_action_local(&f.account, &request, text, key)
                .unwrap()
                .metadata
                .id,
            draft.metadata.id
        );
        let mut other = request.clone();
        other.sns_id = "18".into();
        assert!(
            f.application
                .create_moment_action_local(&f.account, &other, text, key)
                .is_err()
        );
        let reopened =
            Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
        let app = Application::new(reopened, f.provider.clone());
        let preview = app
            .send_preview_local(&f.account, &draft.metadata.id)
            .unwrap();
        assert_eq!(
            preview
                .payload
                .moment
                .as_ref()
                .unwrap()
                .interaction
                .as_ref(),
            Some(&request)
        );
        assert_eq!(preview.payload.text, text);
        f.approve(&draft);
        assert_eq!(
            app.execute_send_local(&f.account, &draft.metadata.id)
                .unwrap()
                .state,
            SendState::Unknown
        );
        assert!(
            app.execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert_eq!(
            app.reconcile_send_local(&f.account, &draft.metadata.id)
                .unwrap()
                .state,
            SendState::Unknown
        );
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn moment_action_rejects_invalid_ids_mixed_payloads_and_utf8_limits() {
    let f = Fixture::new();
    {
        let mut c = f.provider.context.lock().unwrap();
        c.target_id = c.sender_id.clone();
    }
    for id in ["0", "01", "-1", "18446744073709551616"] {
        let action = MomentAction {
            reply: None,
            sns_id: id.into(),
            action: MomentActionKind::Like,
        };
        assert!(
            f.application
                .create_moment_action_local(&f.account, &action, "", "owned-invalid-action")
                .is_err()
        );
    }
    let action = MomentAction {
        reply: None,
        sns_id: "1".into(),
        action: MomentActionKind::Comment,
    };
    for text in [
        "".to_owned(),
        " ".into(),
        "中".repeat(342),
        String::from("a\0b"),
    ] {
        assert!(!action.valid(&text));
    }
    assert!(action.valid(&"中".repeat(341)));
    let mut reply = MomentAction {
        sns_id: "1".into(),
        action: MomentActionKind::Reply,
        reply: Some(MomentReplyTarget {
            username: "owned-user".into(),
            id: Some("42".into()),
            id64: Some(u64::MAX.to_string()),
        }),
    };
    assert!(reply.valid(&"a".repeat(895)));
    assert!(!reply.valid(&"a".repeat(896)));
    reply.reply.as_mut().unwrap().id = Some("042".into());
    assert!(!reply.valid("test"));
    reply.reply.as_mut().unwrap().id = None;
    reply.reply.as_mut().unwrap().id64 = None;
    assert!(!reply.valid("test"));
    reply.reply = None;
    assert!(!reply.valid("test"));
    assert!(
        !MomentAction {
            reply: None,
            sns_id: "1".into(),
            action: MomentActionKind::Like
        }
        .valid("comment")
    );
}

#[tokio::test]
async fn moment_action_api_scope_and_idempotency() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    {
        let mut c = f.provider.context.lock().unwrap();
        c.target_id = c.sender_id.clone();
    }
    let issue = |scope: &str| {
        f.application
            .issue_token_local(
                "互动自有测试",
                vec![f.account.clone()],
                vec![
                    scope.into(),
                    "operations:create".into(),
                    "operations:read".into(),
                ],
                1,
            )
            .unwrap()
    };
    let send = issue("messages:send");
    let moment = issue("moments:publish");
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    let request = |secret: &str, id: &str| {
        Request::builder().method("POST").uri("/api/v1/moments/interact").header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json").header("Idempotency-Key","owned-moment-action-api-001").body(Body::from(serde_json::json!({"account_id":f.account,"action":{"sns_id":id,"action":"comment"},"text":"自动化测试"}).to_string())).unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request(&send.secret, "18446744073709551615"))
            .await
            .unwrap()
            .status(),
        403
    );
    let mut ids = Vec::new();
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(request(&moment.secret, "18446744073709551615"))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(value["data"]["state"], "awaiting_approval");
        assert_eq!(value["data"]["metadata"]["schema_version"], 11);
        ids.push(value["data"]["metadata"]["id"].clone());
    }
    assert_eq!(ids[0], ids[1]);
    assert!(
        !router
            .clone()
            .oneshot(request(&moment.secret, "42"))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let p = f.application.authenticate(&send.secret).unwrap();
    assert!(
        f.application
            .cancel_send(&p, &f.account, ids[0].as_str().unwrap())
            .is_err()
    );
    let status = f
        .repository
        .send_status(&f.account, ids[0].as_str().unwrap())
        .unwrap();
    f.approve(&status);
    f.application.revoke_token_local(&moment.info.id).unwrap();
    assert!(
        f.application
            .execute_send_local(&f.account, &status.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[path = "outbox/video_moments.rs"]
mod video_moments;
