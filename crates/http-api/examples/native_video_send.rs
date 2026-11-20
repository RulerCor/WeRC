//! Explicit local video validation; never run by tests or desktop startup.
#![cfg(windows)]
#[path = "support/client.rs"]
mod client;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppError, AppResult, outbox::TextReceiptQuery};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::native_messages::NativeFileHelperProvider;
fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 8 || !["probe", "send-once"].contains(&a[1].as_str()) {
        return Err(AppError::new(
            "ARGUMENTS",
            "mode host host_hash pid file file_hash new_state_directory",
        ));
    }
    let path = PathBuf::from(&a[7]);
    std::fs::create_dir(&path)
        .map_err(|_| AppError::new("STATE_EXISTS", "必须使用新的隔离状态目录；不自动重复外发"))?;
    let file = zeroize::Zeroizing::new(std::fs::read(&a[5]).map_err(|_| AppError::internal())?);
    if hex::encode(Sha256::digest(&file)) != a[6] {
        return Err(AppError::new("FILE_CHANGED", "核验文件已变化"));
    }
    let repository = Arc::new(SqliteRepository::open(&path.join("state.sqlite3"))?);
    let account = repository.register_account("原生视频隔离验证")?.id;
    let client_path = client::executable(a[4].parse().map_err(|_| AppError::internal())?)?;
    let provider = Arc::new(NativeFileHelperProvider::connect(
        &account,
        &PathBuf::from(&a[2]),
        &a[3],
        a[4].parse().map_err(|_| AppError::internal())?,
        &client_path,
    )?);
    let app = Application::new(repository, provider.clone());
    app.bind_client_local(&account, &client_path)?;
    let filename = PathBuf::from(&a[5])
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(AppError::internal)?
        .to_owned();
    let asset = app.upload_video_local(&account, &filename, file.to_vec())?;
    let draft = app.create_video_send_local(
        &account,
        "filehelper",
        &asset.id,
        "video-validation-single-attempt-20261112",
    )?;
    let preview = app.send_preview_local(&account, &draft.metadata.id)?;
    let prepared = app.video_preview_local(&account, &asset.id)?;
    let mut evidence = serde_json::json!({"started_at":chrono::Utc::now().to_rfc3339(),"mode":a[1],"target":"filehelper","source_file_sha256":a[6],"file_sha256":asset.sha256,"file_size":asset.size,"host_sha256":a[3],"native_object_verified":false,"send_attempted":false,"native_submit_returned_zero":false,"recipient_confirmed":false,"normal_user_store_accessed":false,"complete_release":false});
    let save = |e: &serde_json::Value| {
        std::fs::write(
            path.join("evidence.json"),
            serde_json::to_vec_pretty(e).unwrap(),
        )
        .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    let (verified, seconds) = provider.probe_video(&prepared.file.file_name, &file)?;
    evidence["native_duration_seconds"] = seconds.into();
    evidence["container_duration_ms"] = prepared.info.duration_ms.into();
    save(&evidence)?;
    if !verified {
        return Err(AppError::new(
            "VIDEO_PROBE_FAILED",
            "原生对象验证未通过；没有发送",
        ));
    }
    evidence["native_object_verified"] = true.into();
    save(&evidence)?;
    if a[1] == "send-once" {
        // Explicit automation consent is required; the caller pins the owned
        // test asset and recipient. No fallback, retries or second sends.
        app.approve_text_send_local(
            &account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
        )?;
        evidence["send_attempted"] = true.into();
        save(&evidence)?;
        let mut status = app.execute_send_local(&account, &draft.metadata.id)?;
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let receipt = provider.reconcile_video(
                TextReceiptQuery {
                    outbox_id: &draft.metadata.id,
                    attempt_id: status
                        .attempt_id
                        .as_deref()
                        .ok_or_else(AppError::internal)?,
                    context_digest: &draft.metadata.context_digest,
                    context: &preview.payload.context,
                },
                None,
            )?;
            if receipt.provider_reference.as_deref() == Some("native-video-return:0") {
                evidence["native_submit_returned_zero"] = true.into();
                break;
            }
            if Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        status = app.reconcile_send_local(&account, &draft.metadata.id)?;
        evidence["outbox_state"] = serde_json::to_value(status.state).unwrap();
        let observe_until = Instant::now() + Duration::from_secs(15);
        while Instant::now() < observe_until {
            app.receive_once_local(&account)?;
            std::thread::sleep(Duration::from_millis(200));
        }
        let rows = app.received_texts_local(&account, 0, 100)?;
        evidence["observed_messages"] = rows.len().into();
        evidence["observed_video_messages"] = rows
            .iter()
            .filter(|r| r.payload.message.message_type == 43)
            .count()
            .into();
        save(&evidence)?;
    }
    drop(app);
    drop(provider);
    evidence["controller_closed"] = true.into();
    evidence["finished_at"] = chrono::Utc::now().to_rfc3339().into();
    save(&evidence)?;
    println!("{}", evidence);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1);
    }
}
