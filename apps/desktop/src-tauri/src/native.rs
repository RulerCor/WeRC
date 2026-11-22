use super::*;
use werc_wechat_win::managed_messages::NativeStatus;

struct OwnedReceiver {
    context: werc_domain::outbox::SendContext,
    position: std::sync::atomic::AtomicU64,
}
impl werc_application::ports::CapabilityProvider for OwnedReceiver {
    fn download_received_video(
        &self,
        _: &werc_domain::outbox::SendContext,
        _: &werc_domain::received_media::ReceivedVideoDescriptor,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, AppError> {
        Ok(zeroize::Zeroizing::new(
            include_bytes!("../../../../tests/fixtures/owned-video.mp4").to_vec(),
        ))
    }

    fn download_received_image(
        &self,
        _: &werc_domain::outbox::SendContext,
        _: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, AppError> {
        Ok(zeroize::Zeroizing::new(
            include_bytes!("../../../../tests/fixtures/owned-image.png").to_vec(),
        ))
    }
    fn download_received_file(
        &self,
        _: &werc_domain::outbox::SendContext,
        _: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, AppError> {
        Ok(zeroize::Zeroizing::new(b"owned file".to_vec()))
    }
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        vec![]
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> Result<Vec<u8>, AppError> {
        WindowsWechatProvider.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> Result<Vec<u8>, AppError> {
        WindowsWechatProvider.unprotect_key(c, k)
    }
    fn resolve_send_context(
        &self,
        _: &str,
        t: &str,
    ) -> Result<werc_domain::outbox::SendContext, AppError> {
        if t != "filehelper" {
            return Err(AppError::new("INVALID_TARGET", "自有接收fixture"));
        }
        Ok(self.context.clone())
    }
    fn poll_receive(&self, _: &str) -> Result<werc_domain::receive::ReceiveBatch, AppError> {
        let n = self.position.load(std::sync::atomic::Ordering::SeqCst);
        Ok(werc_domain::receive::ReceiveBatch {
            route: if (2..=6).contains(&n) {
                let conversation = if n == 2 {
                    "owned-contact".to_owned()
                } else {
                    format!("{}@chatroom", "owned-group-".repeat(8))
                };
                Some(werc_domain::receive::ReceivedRoute {
                    sender: conversation.clone(),
                    receiver: self.context.sender_id.clone(),
                    conversation,
                })
            } else {
                None
            },
            inline_voice: if n == 3 {
                let mut wav = Vec::with_capacity(32044);
                wav.extend_from_slice(b"RIFF");
                wav.extend_from_slice(&32036u32.to_le_bytes());
                wav.extend_from_slice(b"WAVEfmt ");
                wav.extend_from_slice(&16u32.to_le_bytes());
                wav.extend_from_slice(&1u16.to_le_bytes());
                wav.extend_from_slice(&1u16.to_le_bytes());
                wav.extend_from_slice(&16000u32.to_le_bytes());
                wav.extend_from_slice(&32000u32.to_le_bytes());
                wav.extend_from_slice(&2u16.to_le_bytes());
                wav.extend_from_slice(&16u16.to_le_bytes());
                wav.extend_from_slice(b"data");
                wav.extend_from_slice(&32000u32.to_le_bytes());
                for i in 0..16000 {
                    let sample = ((i as f64 * 440.0 * std::f64::consts::TAU / 16000.0).sin()
                        * 12000.0) as i16;
                    wav.extend_from_slice(&sample.to_le_bytes());
                }
                Some(werc_wechat_win::voice_codec::encode(&wav)?.bytes)
            } else {
                None
            },
            context: self.context.clone(),
            dropped: 0,
            message: if n <= 6 {
                Some(werc_domain::receive::ReceivedText {
                    message_type: if n == 6 {
                        43
                    } else if n == 5 {
                        49
                    } else if n == 4 {
                        3
                    } else if n == 3 {
                        34
                    } else {
                        1
                    },
                    metadata: if n == 6 {
                        Some(format!(
                            r#"<msg><videomsg aeskey="{}" cdnvideourl="aabbccdd" length="100"/></msg>"#,
                            "ab".repeat(16)
                        ))
                    } else if n == 5 {
                        Some(format!(
                            "<msg><appmsg><title>owned-file.txt</title><type>6</type><appattach><totallen>10</totallen><cdnattachurl>aabbccdd</cdnattachurl><aeskey>{}</aeskey></appattach></appmsg></msg>",
                            "ab".repeat(16)
                        ))
                    } else if n == 4 {
                        Some(format!(
                            "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabbccdd\" length=\"20\"/></msg>",
                            "ab".repeat(16)
                        ))
                    } else if n == 3 {
                        Some("<voicemsg owned_secret=\"never-display-metadata\"/>".into())
                    } else {
                        None
                    },
                    sequence: n,
                    native_message_id: (2000 + n).to_string(),
                    text: if n >= 3 {
                        String::new()
                    } else {
                        format!("自有接收样本 {n} <img src=x onerror=\"window.__nativeXss=true\">")
                    },
                })
            } else {
                None
            },
        })
    }
    fn acknowledge_receive(&self, _: &str, _: &str, n: u64) -> Result<(), AppError> {
        self.position
            .compare_exchange(
                n,
                n + 1,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .map_err(|_| AppError::internal())?;
        Ok(())
    }
}
#[tauri::command]
pub(super) async fn native_fixture(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<(), AppError> {
    if app.config().identifier != "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "仅限自有隔离接收fixture"));
    }
    let binding = state
        .application
        .bindings_local()?
        .into_iter()
        .find(|b| {
            b.account_id == account_id
                && b.installation.product_name == "WeRC synthetic memory target"
        })
        .ok_or_else(|| AppError::new("FIXTURE_REQUIRED", "需要自有客户端样本"))?;
    let repository = state.repository.clone();
    tauri::async_runtime::spawn_blocking(move || {
        use werc_domain::outbox::{SendContext, SendExecutionMode};
        let provider = Arc::new(OwnedReceiver {
            position: std::sync::atomic::AtomicU64::new(1),
            context: SendContext {
                sender_id: "owned-receive-fixture".into(),
                target_id: "filehelper".into(),
                target_display_name: "自有接收fixture".into(),
                identity_verified: true,
                target_verified: true,
                epoch: "owned-native-receive-epoch".into(),
                installation_sha256: binding.installation.sha256.clone(),
                adapter_sha256: binding.installation.sha256,
                execution_mode: SendExecutionMode::Synthetic,
            },
        });
        let application = Application::new(repository, provider);
        for _ in 0..6 {
            application.receive_once_local(&account_id)?;
        }
        let row = application
            .received_texts_local(&account_id, 0, 100)?
            .into_iter()
            .find(|r| r.payload.message.message_type == 3)
            .ok_or_else(AppError::internal)?;
        application.receive_image_file_local(&account_id, row.position)?;
        let file = application
            .received_texts_local(&account_id, 0, 100)?
            .into_iter()
            .find(|r| r.payload.message.message_type == 49)
            .ok_or_else(AppError::internal)?;
        application.receive_file_local(&account_id, file.position)?;
        let video = application
            .received_texts_local(&account_id, 0, 100)?
            .into_iter()
            .find(|r| r.payload.message.message_type == 43)
            .ok_or_else(AppError::internal)?;
        application.receive_video_local(&account_id, video.position)?;
        Ok(())
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn native_processes(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<u32>, AppError> {
    run_local(state, move |app| {
        let binding = app
            .bindings_local()?
            .into_iter()
            .find(|b| b.account_id == account_id)
            .ok_or_else(|| AppError::new("CONNECTION_REQUIRED", "请先绑定客户端"))?;
        let current = app
            .discover_clients_local()?
            .installations
            .into_iter()
            .find(|i| i.id == binding.installation.id);
        let candidates = if let Some(installation) = current {
            if installation.sha256 != binding.installation.sha256 && !installation.process_ids.is_empty() {
                app.refresh_upgraded_binding_local(&account_id, &installation)?;
            }
            installation.process_ids
        } else { Vec::new() };
        let main = werc_wechat_win::setup::main_window_processes(&candidates);
        Ok(if main.is_empty() { candidates } else { main })
    })
    .await
}
#[tauri::command]
pub(super) fn native_status(state: State<'_, DesktopState>) -> Result<NativeStatus, AppError> {
    state.native_provider.status()
}
#[tauri::command]
pub(super) async fn native_current_profile(state:State<'_,DesktopState>,account_id:String,view_id:String,request_id:String)->Result<werc_application::live_contacts::CurrentProfile,AppError>{
    run_local(state,move|app|app.current_profile_local(&account_id,&view_id,&request_id)).await
}
#[tauri::command]
pub(super) async fn native_connect(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    revision: String,
    process_id: u32,
) -> Result<NativeStatus, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不连接真实微信"));
    }
    let application = state.application.clone();
    let provider = state.native_provider.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let slot = application.current_connection_local()?;
        if slot.account_id.as_deref() != Some(&account_id) || slot.revision != revision {
            return Err(AppError::new("CONNECTION_CHANGED", "当前微信配置已变化"));
        }
        let binding = application
            .bindings_local()?
            .into_iter()
            .find(|b| b.account_id == account_id)
            .ok_or_else(|| AppError::new("CONNECTION_REQUIRED", "请先绑定客户端"))?;
        provider.connect(
            &account_id,
            process_id,
            &binding.installation.executable_path,
        )?;
        // Bind authenticated identity and initial gap before exposing readiness.
        if let Err(e) = application.receive_once_local(&account_id) {
            let _ = provider.disconnect(Some(&e.code));
            return Err(e);
        }
        provider.mark_ready(&account_id)?;
        provider.status()
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_disconnect(
    state: State<'_, DesktopState>,
) -> Result<NativeStatus, AppError> {
    let provider = state.native_provider.clone();
    state.application.clear_live_filehelper_local()?;
    tauri::async_runtime::spawn_blocking(move || {
        provider.disconnect(None)?;
        provider.status()
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn native_live_prepare(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    database: String,
    request_id: String,
    confirmed: bool,
) -> Result<werc_application::live_view::LivePage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    let application = state.application.clone();
    let provider = state.native_provider.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let status = provider.status()?;
        if !status.connected || status.account_id.as_deref() != Some(&account_id) {
            return Err(AppError::new(
                "NATIVE_NOT_CONNECTED",
                "请先连接当前微信原生通道",
            ));
        }
        application.bind_live_source_local(&account_id, &database, confirmed)?;
        match application.prepare_live_filehelper_local(
            &account_id,
            &database,
            &request_id,
            confirmed,
        ) {
            Err(e) if matches!(e.code.as_str(), "KEY_REQUIRED" | "KEY_SOURCE_MISMATCH") => {
                application.acquire_database_key_local(
                    &account_id,
                    status.process_id.ok_or_else(AppError::internal)?,
                    &database,
                    confirmed,
                )?;
                application.prepare_live_filehelper_local(
                    &account_id,
                    &database,
                    &request_id,
                    confirmed,
                )
            }
            result => result,
        }
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_live_continue(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    cursor: String,
    request_id: String,
) -> Result<werc_application::live_view::LivePage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    run_local(state, move |app| {
        app.continue_live_filehelper_local(&account_id, &cursor, &request_id)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_live_group_members(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    username: String,
    cursor: Option<String>,
    request_id: String,
) -> Result<werc_application::live_groups::LiveGroupPage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new(
            "FORBIDDEN",
            "隔离桌面测试不读取真实群成员来源",
        ));
    }
    run_local(state, move |app| {
        app.live_group_members_local(
            &account_id,
            &view_id,
            &username,
            cursor.as_deref(),
            &request_id,
        )
    })
    .await
}
#[tauri::command]
pub(super) async fn native_live_contacts(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    cursor: Option<String>,
    search_query: Option<String>,
    request_id: String,
) -> Result<werc_application::live_contacts::LiveContactPage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new(
            "FORBIDDEN",
            "隔离桌面测试不读取真实联系人来源",
        ));
    }
    run_local(state, move |app| {
        app.live_contacts_search_local(
            &account_id,
            &view_id,
            cursor.as_deref(),
            search_query.as_deref(),
            &request_id,
        )
    })
    .await
}

#[tauri::command]
pub(super) async fn native_contact_avatar(state: State<'_,DesktopState>,account_id:String,view_id:String,username:String,request_id:String)->Result<werc_application::contact_avatars::ContactAvatar,AppError>{
    run_local(state,move|app|app.contact_avatar_local(&account_id,&view_id,&username,&request_id)).await
}

#[tauri::command]
pub(super) async fn reveal_database_keys(state:State<'_,DesktopState>,account_id:String)->Result<Vec<werc_application::RevealedDatabaseKey>,AppError>{
    run_local(state,move|app|app.reveal_database_keys_local(&account_id)).await
}
#[tauri::command]
pub(super) async fn native_live_sessions(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    cursor: Option<String>,
    search_query: Option<String>,
    request_id: String,
) -> Result<werc_application::live_sessions::LiveSessionPage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实会话来源"));
    }
    run_local(state, move |app| {
        app.live_sessions_search_local(
            &account_id,
            &view_id,
            cursor.as_deref(),
            search_query.as_deref(),
            &request_id,
        )
    })
    .await
}
#[tauri::command]
pub(super) async fn native_live_moments(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    cursor: Option<String>,
    query: Option<werc_domain::moments::MomentQuery>,
    request_id: String,
) -> Result<werc_application::live_moments::LiveMomentPage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new(
            "FORBIDDEN",
            "隔离桌面测试不读取真实朋友圈来源",
        ));
    }
    run_local(state, move |app| {
        app.query_live_moments_local(&account_id, &view_id, query, cursor.as_deref(), &request_id)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_live_page(
    state: State<'_, DesktopState>,
    account_id: String,
    cursor: Option<String>,
    direction: Option<werc_application::live_view::LiveDirection>,
) -> Result<werc_application::live_view::LivePage, AppError> {
    if cursor.is_some() != direction.is_some() {
        return Err(AppError::new("INVALID_ARGUMENT", "游标与方向必须同时提供"));
    }
    run_local(state, move |app| {
        app.live_filehelper_page_local(&account_id, cursor.as_deref().zip(direction), 60)
    })
    .await
}
#[tauri::command]
pub(super) fn native_live_clear(state: State<'_, DesktopState>) -> Result<(), AppError> {
    state.application.clear_live_filehelper_local()
}
#[tauri::command]
pub(super) async fn native_live_multi_prepare(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    databases: Vec<String>,
    request_id: String,
    confirmed: bool,
) -> Result<werc_application::live_multi_view::MultiLivePage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    let application = state.application.clone();
    let provider = state.native_provider.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let status = provider.status()?;
        if !status.connected || status.account_id.as_deref() != Some(&account_id) {
            return Err(AppError::new(
                "NATIVE_NOT_CONNECTED",
                "请先连接当前微信原生通道",
            ));
        }
        application.prepare_live_filehelper_multi_with_keys_local(
            &account_id,
            &databases,
            status.process_id.ok_or_else(AppError::internal)?,
            &request_id,
            confirmed,
        )
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_live_auto_prepare(app:tauri::AppHandle,state:State<'_,DesktopState>,account_id:String,request_id:String,single:bool)->Result<werc_application::backfill::AutomaticLivePage,AppError>{
    if app.config().identifier=="local.werc.mon3trbot.smoke" {return Err(AppError::new("FORBIDDEN","隔离桌面测试不读取真实微信来源"));}
    run_local(state,move|app|app.prepare_automatic_live_local(&account_id,&request_id,single)).await
}
#[tauri::command]
pub(super) async fn native_live_multi_continue(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    cursor: String,
    request_id: String,
) -> Result<werc_application::live_multi_view::MultiLivePage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    run_local(state, move |app| {
        app.continue_live_filehelper_multi_local(&account_id, &cursor, &request_id)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_live_multi_page(
    state: State<'_, DesktopState>,
    account_id: String,
    cursor: Option<String>,
    direction: Option<werc_application::live_view::LiveDirection>,
) -> Result<werc_application::live_multi_view::MultiLivePage, AppError> {
    if cursor.is_some() != direction.is_some() {
        return Err(AppError::new("INVALID_ARGUMENT", "游标与方向必须同时提供"));
    }
    run_local(state, move |app| {
        app.live_filehelper_multi_page_local(&account_id, cursor.as_deref().zip(direction), 60)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_recovery_save(
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    confirmed: bool,
) -> Result<werc_domain::recovery::RecoverySummary, AppError> {
    run_local(state, move |app| {
        app.recover_live_filehelper_local(&account_id, &view_id, confirmed)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_recovery_multi_save(
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    confirmed: bool,
) -> Result<werc_domain::recovery::RecoverySummary, AppError> {
    run_local(state, move |app| {
        app.recover_live_filehelper_multi_local(&account_id, &view_id, confirmed)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_recovery_list(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<werc_domain::recovery::RecoverySummary>, AppError> {
    run_local(state, move |app| app.recovery_summaries_local(&account_id)).await
}
#[tauri::command]
pub(super) async fn native_recovery_page(
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    offset: usize,
) -> Result<werc_domain::recovery::RecoveryPage, AppError> {
    run_local(state, move |app| {
        app.recovered_filehelper_page_local(&account_id, &id, offset, 60)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_messages(
    state: State<'_, DesktopState>,
    account_id: String,
    after: i64,
    limit: u32,
) -> Result<serde_json::Value, AppError> {
    run_local(state,move|app|Ok(serde_json::json!({"data":app.received_texts_local(&account_id,after,limit)?,"cursor":app.receive_cursor_local(&account_id)?,"data_mode":"native_observations","historical_complete":false}))).await
}
#[tauri::command]
pub(super) async fn native_received_image_asset(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Option<werc_domain::file::FileAsset>, AppError> {
    run_local(state, move |app| {
        app.received_image_file_asset_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_image_download(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<werc_domain::file::FileAsset, AppError> {
    run_local(state, move |app| {
        app.receive_image_file_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_image_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Vec<u8>, AppError> {
    run_local(state, move |app| {
        Ok(app
            .received_image_preview_local(&account_id, position)?
            .to_vec())
    })
    .await
}

#[tauri::command]
pub(super) async fn native_received_voice_asset(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Option<werc_domain::file::FileAsset>, AppError> {
    run_local(state, move |app| {
        app.received_voice_file_asset_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_voice_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Vec<u8>, AppError> {
    run_local(state, move |app| {
        Ok(app
            .received_voice_preview_local(&account_id, position)?
            .to_vec())
    })
    .await
}

#[tauri::command]
pub(super) async fn native_received_file_asset(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Option<werc_domain::file::FileAsset>, AppError> {
    run_local(state, move |app| {
        app.received_file_asset_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_file_download(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<werc_domain::file::FileAsset, AppError> {
    run_local(state, move |app| {
        app.receive_file_local(&account_id, position)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_received_file_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    run_local(state, move |app| {
        let destination = export_file::destination(&path, &downloads)?;
        let output = app.received_file_content_local(&account_id, position)?;
        export_file::write_new(&destination, &output.bytes)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_received_file_info(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Option<werc_domain::received_media::ReceivedFileInfo>, AppError> {
    run_local(state, move |app| {
        app.received_file_info_local(&account_id, position)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_live_file_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    local_id: String,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    run_local(state, move |app| {
        let destination = export_file::destination(&path, &downloads)?;
        let output = app.download_live_file_local(&account_id, &view_id, &local_id)?;
        export_file::write_new(&destination, &output.bytes)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_live_file_info(
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    local_id: String,
) -> Result<werc_domain::received_media::ReceivedFileInfo, AppError> {
    run_local(state, move |app| {
        app.live_file_info_local(&account_id, &view_id, &local_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_live_voice_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    local_id: String,
    request_id: String,
) -> Result<werc_domain::media::AudioAsset, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    run_local(state, move |app| {
        app.save_live_voice_local(&account_id, &view_id, &local_id, &request_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_received_video_asset(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Option<werc_domain::video::VideoAsset>, AppError> {
    run_local(state, move |app| {
        app.received_video_asset_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_video_download(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<werc_domain::video::VideoAsset, AppError> {
    run_local(state, move |app| {
        app.receive_video_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_video_preview(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<tauri::ipc::Response, AppError> {
    run_local(state, move |app| {
        Ok(tauri::ipc::Response::new(
            app.received_video_content_local(&account_id, position)?
                .to_vec(),
        ))
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_video_delete(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<(), AppError> {
    run_local(state, move |app| {
        app.delete_received_video_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_video_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    run_local(state, move |app| {
        let destination = export_file::destination(&path, &downloads)?;
        let bytes = app.received_video_content_local(&account_id, position)?;
        export_file::write_new(&destination, &bytes)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_live_calendar(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    selection: werc_domain::live::LiveCalendarSelection,
    request_id: String,
) -> Result<werc_application::live_calendar::CalendarPage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.live_calendar_local(&account_id, &view_id, &selection, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_live_search(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    query: Option<werc_domain::live::LiveMessageQuery>,
    cursor: Option<String>,
    request_id: String,
) -> Result<werc_application::live_search::SearchPage, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.search_live_messages_local(&account_id, &view_id, query, cursor.as_deref(), &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_live_conversation(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    command: werc_application::live_conversation::ConversationCommand,
    request_id: String,
) -> Result<Option<werc_application::live_conversation::ConversationPage>, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.live_conversation_local(&account_id, command, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn native_live_multi_conversation(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    command: werc_application::live_conversation::ConversationCommand,
    request_id: String,
) -> Result<Option<werc_application::live_multi_conversation::MultiConversationPage>, AppError> {
    if app.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离测试不访问真实来源"));
    }
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.live_multi_conversation_local(&account_id, command, &request_id)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn native_live_emoticon_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    view_id: String,
    local_id: String,
    request_id: String,
) -> Result<werc_domain::file::FileAsset, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    run_local(state, move |app| {
        app.save_live_emoticon_local(&account_id, &view_id, &local_id, &request_id)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_received_emoticon_asset(
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
) -> Result<Option<werc_domain::file::FileAsset>, AppError> {
    run_local(state, move |app| {
        app.received_emoticon_asset_local(&account_id, position)
    })
    .await
}
#[tauri::command]
pub(super) async fn native_received_emoticon_read(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    position: i64,
    request_id: String,
) -> Result<werc_domain::file::FileAsset, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    run_local(state, move |app| {
        app.receive_emoticon_local(&account_id, position, &request_id)
    })
    .await
}
#[tauri::command]
pub(super) async fn export_emoticon_content(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    id: String,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    run_local(state, move |app| {
        let destination = export_file::destination(&path, &downloads)?;
        let output = app.emoticon_contents_local(&account_id, &id)?;
        let extension = if output.file_name.ends_with(".png") {
            "png"
        } else {
            "gif"
        };
        if !destination
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case(extension))
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                &format!("此动画请使用 .{extension} 扩展名"),
            ));
        }
        export_file::write_new(&destination, &output.bytes)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_conversation_media_save(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    selection: werc_application::conversation_media::ConversationMediaSelection,
    forward_index: Option<usize>,
    forward_path: Option<Vec<usize>>,
    request_id: String,
) -> Result<werc_application::conversation_media::ConversationMediaAsset, AppError> {
    if handle.config().identifier == "local.werc.mon3trbot.smoke" {
        return Err(AppError::new("FORBIDDEN", "隔离桌面测试不读取真实微信来源"));
    }
    run_local(state, move |app| {
        app.save_conversation_media_path_local(
            &account_id,
            &selection,
            forward_index,
            forward_path.as_deref(),
            &request_id,
        )
    })
    .await
}
#[tauri::command]
pub(super) async fn native_conversation_media_content(
    state: State<'_, DesktopState>,
    account_id: String,
    reference: werc_application::conversation_media::ConversationMediaRef,
) -> Result<tauri::ipc::Response, AppError> {
    run_local(state, move |app| {
        Ok(tauri::ipc::Response::new(
            app.conversation_media_content_local(&account_id, &reference)?
                .file
                .bytes
                .to_vec(),
        ))
    })
    .await
}
#[tauri::command]
pub(super) async fn native_conversation_media_delete(
    state: State<'_, DesktopState>,
    account_id: String,
    reference: werc_application::conversation_media::ConversationMediaRef,
) -> Result<(), AppError> {
    run_local(state, move |app| {
        app.delete_conversation_media_local(&account_id, &reference)
    })
    .await
}
#[tauri::command]
pub(super) async fn export_conversation_media(
    handle: tauri::AppHandle,
    state: State<'_, DesktopState>,
    account_id: String,
    reference: werc_application::conversation_media::ConversationMediaRef,
    path: String,
) -> Result<usize, AppError> {
    let downloads = handle
        .path()
        .download_dir()
        .map_err(|_| AppError::new("EXPORT_DIRECTORY_UNAVAILABLE", "无法定位系统下载目录"))?;
    run_local(state, move |app| {
        let output = app.conversation_media_content_local(&account_id, &reference)?;
        let name = if path.trim().is_empty() {
            &output.file.file_name
        } else {
            &path
        };
        let destination = export_file::destination(name, &downloads)?;
        if reference.kind != werc_application::conversation_media::ConversationMediaKind::File {
            let extension = std::path::Path::new(&output.file.file_name)
                .extension()
                .and_then(|s| s.to_str())
                .ok_or_else(AppError::internal)?;
            if !destination
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case(extension))
            {
                return Err(AppError::new(
                    "INVALID_ARGUMENT",
                    &format!("请使用 .{extension} 扩展名"),
                ));
            }
        }
        export_file::write_new(&destination, &output.file.bytes)
    })
    .await
}

#[tauri::command]
pub(super) async fn native_prepared_recipients(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<Vec<werc_domain::outbox::PreparedRecipient>, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.prepared_recipients_local(&account_id))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_prepare_recipient(
    state: State<'_, DesktopState>,
    account_id: String,
    selection: werc_application::recipients::RecipientSelection,
) -> Result<werc_domain::outbox::PreparedRecipient, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.prepare_recipient_local(&account_id, selection)
    })
    .await
    .map_err(|_| AppError::internal())?
}

#[tauri::command]
pub(super) async fn native_backfill_status(
    state: State<'_, DesktopState>,
    account_id: String,
) -> Result<werc_application::backfill::BackfillStatus, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.backfill_status_local(&account_id))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn native_backfill_page(
    state: State<'_, DesktopState>,
    account_id: String,
    after: u64,
) -> Result<werc_application::backfill::BackfillPage, AppError> {
    let app = state.application.clone();
    tauri::async_runtime::spawn_blocking(move || app.backfill_page_local(&account_id, after, 1))
        .await
        .map_err(|_| AppError::internal())?
}
