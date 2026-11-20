//! Explicit local voice validation; never run by tests or desktop startup.
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
    if a.len() != 10 || !["probe", "send-once"].contains(&a[1].as_str()) {
        return Err(AppError::new(
            "ARGUMENTS",
            "mode host host_hash codec codec_hash pid wav wav_hash new_state_directory",
        ));
    }
    let path = PathBuf::from(&a[9]);
    std::fs::create_dir(&path)
        .map_err(|_| AppError::new("STATE_EXISTS", "必须使用新的隔离状态目录；不自动重复外发"))?;
    let wav = zeroize::Zeroizing::new(std::fs::read(&a[7]).map_err(|_| AppError::internal())?);
    if hex::encode(Sha256::digest(&wav)) != a[8] {
        return Err(AppError::new("AUDIO_CHANGED", "核验音频已变化"));
    }
    let repository = Arc::new(SqliteRepository::open(&path.join("state.sqlite3"))?);
    let account = repository.register_account("原生语音隔离验证")?.id;
    let client_path = client::executable(a[6].parse().map_err(|_| AppError::internal())?)?;
    let provider = Arc::new(
        NativeFileHelperProvider::connect(
            &account,
            &PathBuf::from(&a[2]),
            &a[3],
            a[6].parse().map_err(|_| AppError::internal())?,
            &client_path,
        )?
        .with_voice_codec(PathBuf::from(&a[4]), a[5].clone()),
    );
    let app = Application::new(repository, provider.clone());
    app.bind_client_local(&account, &client_path)?;
    let asset = app.upload_audio_local(&account, wav.to_vec())?;
    let draft = app.create_voice_send_local(
        &account,
        "filehelper",
        &asset.id,
        "voice-validation-single-attempt-20261111",
    )?;
    let preview = app.send_preview_local(&account, &draft.metadata.id)?;
    let encoded = provider.encode_voice(&wav)?;
    let mut evidence = serde_json::json!({"started_at":chrono::Utc::now().to_rfc3339(),"mode":a[1],"target":"filehelper","wav_sha256":a[8],"codec_sha256":a[5],"host_sha256":a[3],"silk_sha256":hex::encode(Sha256::digest(&encoded.bytes)),"duration_ms":encoded.duration_ms,"native_object_verified":false,"send_attempted":false,"native_submit_returned_zero":false,"recipient_confirmed":false,"normal_user_store_accessed":false,"complete_release":false});
    let save = |e: &serde_json::Value| {
        std::fs::write(
            path.join("evidence.json"),
            serde_json::to_vec_pretty(e).unwrap(),
        )
        .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    if !provider.probe_voice(&encoded)? {
        return Err(AppError::new(
            "VOICE_PROBE_FAILED",
            "原生对象验证未通过；没有发送",
        ));
    }
    evidence["native_object_verified"] = true.into();
    save(&evidence)?;
    if a[1] == "send-once" {
        // This explicit mode requires separate human approval of the concrete
        // wav/hash/recipient. No text, file fallback, retries or second sends.
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
            let receipt = provider.reconcile_voice(
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
            if receipt.provider_reference.as_deref() == Some("native-voice-return:0") {
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
