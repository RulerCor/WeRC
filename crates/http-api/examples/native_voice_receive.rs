//! Explicit receive-only, isolated encrypted state. No send/approval operations.
#![cfg(windows)]
#[path = "support/client.rs"]
mod client;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{
    WindowsWechatProvider, native_messages::NativeFileHelperProvider, voice_codec,
};
fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 8 {
        return Err(AppError::new(
            "ARGUMENTS",
            "host hash pid fresh_directory seconds codec codec_hash",
        ));
    }
    let seconds: u64 = a[5].parse().map_err(|_| AppError::internal())?;
    if !(1..=600).contains(&seconds) {
        return Err(AppError::internal());
    }
    let path = PathBuf::from(&a[4]);
    std::fs::create_dir(&path)
        .map_err(|_| AppError::new("STATE_EXISTS", "fresh directory required"))?;
    let db = path.join("state.sqlite3");
    let repo = Arc::new(SqliteRepository::open(&db)?);
    let account = repo.register_account("语音内容只接收隔离验证")?.id;
    let client_path = client::executable(a[3].parse().map_err(|_| AppError::internal())?)?;
    let provider = Arc::new(NativeFileHelperProvider::connect(
        &account,
        &PathBuf::from(&a[1]),
        &a[2],
        a[3].parse().map_err(|_| AppError::internal())?,
        &client_path,
    )?);
    let app = Application::new(repo.clone(), provider.clone());
    app.bind_client_local(&account, &client_path)?;
    let mut evidence = serde_json::json!({"started_at":chrono::Utc::now().to_rfc3339(),"abi":werc_wechat_win::native_messages::NATIVE_PROTOCOL_VERSION,"schema":18,"host_sha256":a[2],"codec_sha256":a[7],"listening":true,"messages_sent":false,"normal_user_store_accessed":false,"complete_release":false});
    let save = |v: &serde_json::Value| {
        std::fs::write(
            path.join("evidence.json"),
            serde_json::to_vec_pretty(v).unwrap(),
        )
        .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    println!("native_voice_listener_ready");
    let until = Instant::now() + Duration::from_secs(seconds);
    let mut position = None;
    while Instant::now() < until {
        if let Err(e) = app.receive_once_local(&account) {
            evidence["receive_error"] = e.code.into();
            break;
        }
        let rows = app.received_texts_local(&account, 0, 100)?;
        evidence["observations"] = rows.len().into();
        if let Some(row) = rows.iter().find(|r| r.payload.message.message_type == 34) {
            position = Some(row.position);
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    evidence["voice_observed"] = position.is_some().into();
    evidence["gap_count"] = app
        .receive_cursor_local(&account)?
        .map_or(0, |c| c.gaps)
        .into();
    evidence["outbox_empty"] = app.sends_local(&account)?.is_empty().into();
    let mut original = None;
    if let Some(pos) = position {
        match app.received_voice_silk_local(&account, pos) {
            Ok(silk) => {
                evidence["inline_bytes"] = silk.len().into();
                let sha = hex::encode(Sha256::digest(&silk));
                evidence["silk_sha256"] = sha.clone().into();
                match voice_codec::decode_at(&PathBuf::from(&a[6]), &a[7], &silk) {
                    Ok(wav) => {
                        let info = werc_domain::media::inspect_wav(&wav)?;
                        let wav_sha = hex::encode(Sha256::digest(&wav));
                        evidence["decoded"] = true.into();
                        evidence["duration_ms"] = info.duration_ms.into();
                        evidence["wav_bytes"] = wav.len().into();
                        evidence["wav_sha256"] = wav_sha.clone().into();
                        original = Some((sha, wav_sha));
                    }
                    Err(e) => {
                        evidence["decode_error"] = e.code.into();
                    }
                }
            }
            Err(e) => {
                evidence["inline_error"] = e.code.into();
            }
        }
    }
    drop(app);
    drop(provider);
    drop(repo);
    evidence["controller_closed"] = true.into();
    evidence["listening"] = false.into();
    save(&evidence)?;
    if let (Some(pos), Some((sha, wav_sha))) = (position, original) {
        let repo = Arc::new(SqliteRepository::open(&db)?);
        let offline = Application::new(repo, Arc::new(WindowsWechatProvider));
        let silk = offline.received_voice_silk_local(&account, pos)?;
        let wav = voice_codec::decode_at(&PathBuf::from(&a[6]), &a[7], &silk)?;
        evidence["offline_reopen_silk_matched"] =
            (hex::encode(Sha256::digest(&silk)) == sha).into();
        evidence["offline_reopen_wav_matched"] =
            (hex::encode(Sha256::digest(&wav)) == wav_sha).into();
    }
    evidence["encrypted_research_state_retained"] = true.into();
    evidence["finished_at"] = chrono::Utc::now().to_rfc3339().into();
    save(&evidence)?;
    println!("{evidence}");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1);
    }
}
