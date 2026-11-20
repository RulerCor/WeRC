//! Explicit local real-client test. Never run by cargo test or normal startup.
//! One immutable test message, persisted through the existing encrypted outbox.
#![cfg(windows)]
#[path = "support/client.rs"]
mod client;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppError, AppResult, outbox::SendExecutionMode};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::native_messages::NativeFileHelperProvider;
const SEND_TEXT: &str = "自动化测试：WeRC 原生任务完成模型编号核验 2026-11-13（仅一次）";
const RECEIVE_TEXT: &str = "自动化测试：WeRC 接收验证";
fn run() -> AppResult<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 7 || !["connect", "send-once"].contains(&args[1].as_str()) {
        return Err(AppError::new(
            "ARGUMENTS",
            "mode host host_sha256 pid new_state_directory seconds",
        ));
    }
    let pid = args[4]
        .parse()
        .map_err(|_| AppError::new("ARGUMENTS", "invalid pid"))?;
    let seconds: u64 = args[6]
        .parse()
        .map_err(|_| AppError::new("ARGUMENTS", "invalid seconds"))?;
    if seconds > 600 {
        return Err(AppError::new("ARGUMENTS", "max 600 seconds"));
    }
    let directory = PathBuf::from(&args[5]);
    std::fs::create_dir(&directory)
        .map_err(|_| AppError::new("STATE_EXISTS", "测试状态目录必须新建；已有发送不能自动重复"))?;
    let repository = Arc::new(SqliteRepository::open(&directory.join("state.sqlite3"))?);
    let account = repository.register_account("原生filehelper隔离验证")?.id;
    let client_path = client::executable(pid)?;
    let provider = Arc::new(NativeFileHelperProvider::connect(
        &account,
        &PathBuf::from(&args[2]),
        &args[3],
        pid,
        &client_path,
    )?);
    let application = Application::new(repository.clone(), provider.clone());
    application.bind_client_local(&account, &client_path)?;
    let context = provider.resolve_send_context(&account, "filehelper")?;
    if !context.identity_verified
        || !context.target_verified
        || context.execution_mode != SendExecutionMode::Native
    {
        return Err(AppError::new("PREFLIGHT_FAILED", "身份或目标未验真"));
    }
    println!(
        "{{\"stage\":\"connected\",\"identity_verified\":true,\"target\":\"filehelper\",\"normal_user_store_accessed\":false}}"
    );
    let mut evidence = serde_json::json!({"schema_version":1,"started_at":chrono::Utc::now().to_rfc3339(),
        "identity_verified":true,"target":"filehelper","source_built":true,"execution_mode":"native",
        "send_attempted":false,"native_callback_success":false,"recipient_confirmed":false,
        "matching_inbound_received":false,"server_message_id_present":false,"gap_observed":false,
        "normal_user_store_accessed":false,"complete_release":false});
    let evidence_path = directory.join("evidence.json");
    let save = |e: &serde_json::Value| -> AppResult<()> {
        std::fs::write(&evidence_path, serde_json::to_vec_pretty(e).unwrap())
            .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    let mut send_id = None;
    if args[1] == "send-once" {
        let draft = application.create_text_send_local(
            &account,
            "filehelper",
            SEND_TEXT,
            "native-text-task-final-20261113-once",
        )?;
        let preview = application.send_preview_local(&account, &draft.metadata.id)?;
        if preview.payload.text != SEND_TEXT || preview.payload.context != context {
            return Err(AppError::new("PREVIEW_CHANGED", "发送预览不符"));
        }
        application.approve_text_send_local(
            &account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
        )?;
        evidence["send_attempted"] = true.into();
        save(&evidence)?;
        let status = application.execute_send_local(&account, &draft.metadata.id)?;
        evidence["outbox_state"] = serde_json::to_value(status.state).unwrap();
        save(&evidence)?;
        println!(
            "{}",
            serde_json::json!({"stage":"submitted","outbox_state":status.state,"automatic_retry":false})
        );
        send_id = Some(draft.metadata.id);
    }
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut callback_reported = false;
    let mut receive_position = 0;
    while Instant::now() < deadline {
        match application.receive_once_local(&account) {
            Ok(progress) => {
                evidence["unproven_intervals"] = progress.gaps.into();
                evidence["gap_observed"] = (progress.gaps > 0).into();
                for row in application.received_texts_local(&account, receive_position, 100)? {
                    receive_position = row.position;
                    if row.payload.message.text == RECEIVE_TEXT {
                        evidence["matching_inbound_received"] = true.into();
                        evidence["server_message_id_present"] =
                            (row.payload.message.native_message_id != "0").into();
                        evidence["inbound_persisted_encrypted"] = true.into();
                        evidence["inbound_acknowledged"] = progress.acknowledged.into();
                        save(&evidence)?;
                        println!(
                            "{{\"stage\":\"inbound_test_observed\",\"body_not_logged\":true}}"
                        );
                    }
                }
            }
            Err(e) => {
                evidence["receive_error"] = e.code.clone().into();
                save(&evidence)?;
                return Err(e);
            }
        }
        if let Some(id) = &send_id
            && !callback_reported
        {
            let status = application.reconcile_send_local(&account, id)?;
            let preview = application.send_preview_local(&account, id)?;
            let receipt = provider.reconcile_text(
                werc_domain::outbox::TextReceiptQuery {
                    outbox_id: id,
                    attempt_id: status
                        .attempt_id
                        .as_deref()
                        .ok_or_else(AppError::internal)?,
                    context_digest: &status.metadata.context_digest,
                    context: &preview.payload.context,
                },
                None,
            )?;
            if receipt.provider_reference.as_deref() == Some("native-callback:2")
                && !callback_reported
            {
                callback_reported = true;
                let (observed, stage) = provider.observed_text_message_id(
                    &preview.payload.context,
                    status
                        .attempt_id
                        .as_deref()
                        .ok_or_else(AppError::internal)?,
                )?;
                evidence["object_observation_stage"] = stage.into();
                evidence["outgoing_object_id_present"] = observed.is_some().into();
                // The candidate is test-owned, bound to this one attempt. Retained
                // locally to permit exact readback, never interpreted as delivery.
                evidence["outgoing_object_id"] = observed.map(|v| v.to_string()).into();
                evidence["native_callback_success"] = true.into();
                save(&evidence)?;
                println!("{{\"stage\":\"native_callback_success\",\"outbox_still_unknown\":true}}");
            }
        }
        if callback_reported {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    drop(application);
    drop(provider);
    evidence["controller_closed"] = true.into();
    evidence["finished_at"] = chrono::Utc::now().to_rfc3339().into();
    save(&evidence)?;
    println!("{}", evidence);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!(
            "{}",
            serde_json::json!({"error":e.code,"message":e.message})
        );
        std::process::exit(1);
    }
}
