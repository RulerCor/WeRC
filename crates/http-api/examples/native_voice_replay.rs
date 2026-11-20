//! Offline decode of an explicitly supplied isolated encrypted receive test store.
//! No native attach, send, network, audio device, or plaintext output.
#![cfg(windows)]
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use werc_application::Application;
use werc_domain::{AppError, AppResult};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{WindowsWechatProvider, voice_codec};
fn inspect(
    db: &std::path::Path,
    codec: &std::path::Path,
    hash: &str,
) -> AppResult<serde_json::Value> {
    let repo = Arc::new(SqliteRepository::open(db)?);
    let app = Application::new(repo, Arc::new(WindowsWechatProvider));
    let accounts = app.accounts_local()?;
    if accounts.len() != 1 {
        return Err(AppError::internal());
    }
    let account = &accounts[0].id;
    let rows = app.received_texts_local(account, 0, 100)?;
    let mut records = Vec::new();
    for row in rows.iter().filter(|r| r.payload.message.message_type == 34) {
        let silk = app.received_voice_silk_local(account, row.position)?;
        let wav = voice_codec::decode_at(codec, hash, &silk)?;
        let info = werc_domain::media::inspect_wav(&wav)?;
        records.push(serde_json::json!({"silk_bytes":silk.len(),"silk_sha256":hex::encode(Sha256::digest(&silk)),"wav_bytes":wav.len(),"wav_sha256":hex::encode(Sha256::digest(&wav)),"duration_ms":info.duration_ms,"sample_rate":info.sample_rate,"channels":info.channels}));
    }
    Ok(
        serde_json::json!({"observations":rows.len(),"voices":records,"outbox_empty":app.sends_local(account)?.is_empty(),"gap_count":app.receive_cursor_local(account)?.map_or(0,|c|c.gaps)}),
    )
}
fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 5 {
        return Err(AppError::new(
            "ARGUMENTS",
            "isolated_store codec codec_hash fresh_output_json",
        ));
    }
    let db = PathBuf::from(&a[1]);
    let codec = PathBuf::from(&a[2]);
    let output = PathBuf::from(&a[4]);
    if !db.is_file() || output.exists() {
        return Err(AppError::internal());
    }
    let first = inspect(&db, &codec, &a[3])?;
    let again = inspect(&db, &codec, &a[3])?;
    let matched = first == again;
    let count = first["voices"]
        .as_array()
        .ok_or_else(AppError::internal)?
        .len();
    let report = serde_json::json!({"checked_at":chrono::Utc::now().to_rfc3339(),"offline":true,"messages_sent":false,"normal_user_store_accessed":false,"codec_sha256":a[3],"decoded_voice_count":count,"closed_store_reopen_matched":matched,"result":first,"passed":count>0&&matched,"complete_release":false});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap())
        .map_err(|_| AppError::internal())?;
    println!("{report}");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1);
    }
}
