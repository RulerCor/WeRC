use super::*;
use werc_domain::outbox::{SendPreview, SendStatus};
#[tauri::command]
pub(super) async fn list_send_history(
    state: State<'_, DesktopState>,
    account_id: String,
    before: Option<i64>,
) -> Result<werc_domain::outbox::SendPage, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.send_history_page_local(&account_id, before, 100)
    })
    .await
    .map_err(|_| AppError::internal())?
}

// 只用于隔离 WebView2 回归。此提供方没有 submit_text 实现，不能调用微信。
struct OwnedDraft(werc_domain::outbox::SendContext);
impl werc_application::ports::CapabilityProvider for OwnedDraft {
    fn encode_voice(&self, wav: &[u8]) -> Result<werc_domain::voice::EncodedVoice, AppError> {
        werc_wechat_win::voice_codec::encode(wav)
    }
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        vec![]
    }
    fn protect_key(&self, context: &str, key: &[u8]) -> Result<Vec<u8>, AppError> {
        WindowsWechatProvider.protect_key(context, key)
    }
    fn unprotect_key(&self, context: &str, key: &[u8]) -> Result<Vec<u8>, AppError> {
        WindowsWechatProvider.unprotect_key(context, key)
    }
    fn resolve_send_context(
        &self,
        _account: &str,
        target: &str,
    ) -> Result<werc_domain::outbox::SendContext, AppError> {
        if target != "owned-fixture-recipient" {
            return Err(AppError::new("INVALID_TARGET", "只允许自有模拟目标"));
        }
        Ok(self.0.clone())
    }
}
#[tauri::command]
pub(super) async fn send_fixture(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    key: String,
    unknown: bool,
    media: Option<String>,
) -> Result<SendStatus, AppError> {
    use werc_application::ports::Repository;
    use werc_domain::outbox::{SendContext, SendExecutionMode};
    if app.config().identifier != "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "自有草稿仅限隔离测试标识"));
    }
    if !matches!(
        media.as_deref(),
        None | Some("file" | "image" | "voice" | "video" | "emoticon")
    ) {
        return Err(AppError::new("INVALID_ARGUMENT", "未知自有草稿类型"));
    }
    let binding = state
        .application
        .bindings_local()?
        .into_iter()
        .find(|b| {
            b.account_id == account_id
                && b.installation.product_name == "WeRC synthetic memory target"
        })
        .ok_or_else(|| AppError::new("FIXTURE_REQUIRED", "仅允许已绑定的自有PE样本"))?;
    let provider = OwnedDraft(SendContext {
        sender_id: "owned-fixture-sender".into(),
        target_id: "owned-fixture-recipient".into(),
        target_display_name: "自有模拟接收者 / 非微信".into(),
        identity_verified: true,
        target_verified: true,
        epoch: "owned-static-epoch".into(),
        installation_sha256: binding.installation.sha256.clone(),
        adapter_sha256: binding.installation.sha256,
        execution_mode: SendExecutionMode::Synthetic,
    });
    let repository = state.repository.clone();
    // 先在真实 DPAPI/SQLite 链路生成草稿，不装配到正常执行器，也不调用 submit_text。
    tauri::async_runtime::spawn_blocking(move || {
        let application = Application::new(repository.clone(), Arc::new(provider));
        let draft = if media.as_deref() == Some("emoticon") {
            let asset = application.upload_emoticon_local(
                &account_id,
                "自动化测试.gif",
                include_bytes!("../../../../tests/fixtures/owned-send-emoticon.gif").to_vec(),
            )?;
            application.create_emoticon_send_local(
                &account_id,
                "owned-fixture-recipient",
                &asset.id,
                &key,
            )?
        } else if media.as_deref() == Some("video") {
            let asset = application.upload_video_local(
                &account_id,
                "自动化测试.mp4",
                include_bytes!("../../../../tests/fixtures/owned-send-video.mp4").to_vec(),
            )?;
            application.create_video_send_local(
                &account_id,
                "owned-fixture-recipient",
                &asset.id,
                &key,
            )?
        } else if media.as_deref() == Some("file") {
            let asset = application.upload_file_local(
                &account_id,
                "自有附件验证.txt",
                b"owned desktop attachment\n".to_vec(),
            )?;
            application.create_file_send_local(
                &account_id,
                "owned-fixture-recipient",
                &asset.id,
                &key,
            )?
        } else if media.as_deref() == Some("image") {
            let asset = application.upload_image_local(
                &account_id,
                include_bytes!("../../../../tests/fixtures/owned-image.png").to_vec(),
            )?;
            application.create_image_send_local(
                &account_id,
                "owned-fixture-recipient",
                &asset.id,
                &key,
            )?
        } else if media.as_deref() == Some("voice") {
            let mut wav = b"RIFF".to_vec();
            wav.extend(676u32.to_le_bytes());
            wav.extend(b"WAVEfmt ");
            wav.extend(16u32.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(16000u32.to_le_bytes());
            wav.extend(32000u32.to_le_bytes());
            wav.extend(2u16.to_le_bytes());
            wav.extend(16u16.to_le_bytes());
            wav.extend(b"data");
            wav.extend(640u32.to_le_bytes());
            wav.extend(vec![0u8; 640]);
            let asset = application.upload_audio_local(&account_id, wav)?;
            application.create_voice_send_local(
                &account_id,
                "owned-fixture-recipient",
                &asset.id,
                &key,
            )?
        } else {
            application.create_text_send_local(
                &account_id,
                "owned-fixture-recipient",
                "<img src=x onerror=\"window.__outboxXss=true\">\n自有文字草稿；不会发送微信。",
                &key,
            )?
        };
        if unknown {
            let approved = application.approve_text_send_local(
                &account_id,
                &draft.metadata.id,
                &draft.metadata.request_digest,
                true,
            )?;
            repository.start_send(&approved, &draft.metadata.id)?;
            repository.mark_send_unknown(&account_id, &draft.metadata.id, &draft.metadata.id)
        } else {
            Ok(draft)
        }
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn list_sends(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<SendStatus>, AppError> {
    run_local(state, move |app| app.sends_local(&account_id)).await
}
#[tauri::command]
pub(super) async fn preview_send(
    state: State<'_, DesktopState>,
    account_id: String,
    outbox_id: String,
) -> Result<SendPreview, AppError> {
    run_local(state, move |app| {
        app.send_preview_local(&account_id, &outbox_id)
    })
    .await
}
#[tauri::command]
pub(super) async fn create_text_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    target_id: String,
    text: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_text_send_local(&account_id, &target_id, &text, &idempotency_key)
    })
    .await
}

#[tauri::command]
pub(super) async fn create_mention_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    selection: werc_application::mention_send::MentionSelection,
    text: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_mention_send_local(&account_id, selection, &text, &idempotency_key)
    })
    .await
}

#[tauri::command]
pub(super) async fn create_voice_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    target_id: String,
    asset_id: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_voice_send_local(&account_id, &target_id, &asset_id, &idempotency_key)
    })
    .await
}
#[tauri::command]
pub(super) async fn create_image_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    target_id: String,
    asset_id: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_image_send_local(&account_id, &target_id, &asset_id, &idempotency_key)
    })
    .await
}
#[tauri::command]
pub(super) async fn create_file_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    target_id: String,
    asset_id: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_file_send_local(&account_id, &target_id, &asset_id, &idempotency_key)
    })
    .await
}
#[tauri::command]
pub(super) async fn approve_send(
    state: State<'_, DesktopState>,
    account_id: String,
    outbox_id: String,
    request_digest: String,
    confirmed: bool,
    scheduled_at: Option<i64>,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.approve_timed_send_local(
            &account_id,
            &outbox_id,
            &request_digest,
            confirmed,
            scheduled_at,
        )
    })
    .await
}
#[tauri::command]
pub(super) async fn cancel_send(
    state: State<'_, DesktopState>,
    account_id: String,
    outbox_id: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.cancel_send_local(&account_id, &outbox_id)
    })
    .await
}
#[tauri::command]
pub(super) async fn reconcile_send(
    state: State<'_, DesktopState>,
    account_id: String,
    outbox_id: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.reconcile_send_local(&account_id, &outbox_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn create_video_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    target_id: String,
    asset_id: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_video_send_local(&account_id, &target_id, &asset_id, &idempotency_key)
    })
    .await
}

#[tauri::command]
pub(super) async fn create_emoticon_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    target_id: String,
    asset_id: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_emoticon_send_local(&account_id, &target_id, &asset_id, &idempotency_key)
    })
    .await
}

#[tauri::command]
pub(super) async fn create_quote_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    selection: werc_application::conversation_media::ConversationMediaSelection,
    text: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_quote_send_local(&account_id, &selection, &text, &idempotency_key)
    })
    .await
}

#[tauri::command]
pub(super) async fn create_private_moment_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    image_asset_ids: Option<Vec<String>>,
    text: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_private_image_moment_local(
            &account_id,
            &text,
            &image_asset_ids.unwrap_or_default(),
            &idempotency_key,
        )
    })
    .await
}

#[tauri::command]
pub(super) async fn create_moment_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    image_asset_ids: Option<Vec<String>>,
    video_asset_id: Option<String>,
    text: String,
    visibility: werc_domain::outbox::MomentVisibility,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        let images = image_asset_ids.unwrap_or_default();
        if let Some(video) = video_asset_id {
            if images.len() != 1 { return Err(AppError::new("INVALID_ARGUMENT", "视频动态需要一张封面")); }
            return app.create_video_moment_local(&account_id, &text, &video, &images[0], visibility, &idempotency_key);
        }
        app.create_moment_local(
            &account_id,
            &text,
            &images,
            visibility,
            &idempotency_key,
        )
    })
    .await
}

#[tauri::command]
pub(super) async fn create_moment_action_draft(
    state: State<'_, DesktopState>,
    account_id: String,
    action: werc_domain::outbox::MomentAction,
    text: String,
    idempotency_key: String,
) -> Result<SendStatus, AppError> {
    run_local(state, move |app| {
        app.create_moment_action_local(&account_id, &action, &text, &idempotency_key)
    })
    .await
}
