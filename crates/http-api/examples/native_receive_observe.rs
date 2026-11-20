//! Explicit read-only native observation. No send or approval calls exist here.
#![cfg(windows)]
#[path = "support/client.rs"]
mod client;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{WindowsWechatProvider, native_messages::NativeFileHelperProvider};
fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 6 {
        return Err(AppError::new(
            "ARGUMENTS",
            "host host_hash pid new_directory seconds",
        ));
    }
    let seconds: u64 = a[5].parse().map_err(|_| AppError::internal())?;
    if !(1..=600).contains(&seconds) {
        return Err(AppError::new("ARGUMENTS", "duration 1..600"));
    }
    let path = PathBuf::from(&a[4]);
    std::fs::create_dir(&path)
        .map_err(|_| AppError::new("STATE_EXISTS", "fresh isolated directory required"))?;
    let db = path.join("state.sqlite3");
    let repo = Arc::new(SqliteRepository::open(&db)?);
    let account = repo.register_account("原生只接收隔离验证")?.id;
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
    let mut evidence = serde_json::json!({"started_at":chrono::Utc::now().to_rfc3339(),"host_sha256":a[2],"abi":werc_wechat_win::native_messages::NATIVE_PROTOCOL_VERSION,"listening":true,"messages_sent":false,"normal_user_store_accessed":false,"observations":0,"types":{},"durable_reopen_matched":false,"media_bytes_downloaded":false,"complete_release":false});
    let save = |v: &serde_json::Value| {
        std::fs::write(
            path.join("evidence.json"),
            serde_json::to_vec_pretty(v).unwrap(),
        )
        .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    println!("native_receive_listener_ready");
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        app.receive_once_local(&account)?;
        std::thread::sleep(Duration::from_millis(250));
    }
    let rows = app.received_texts_local(&account, 0, 100)?;
    let mut kinds = BTreeMap::<u32, usize>::new();
    for row in &rows {
        *kinds.entry(row.payload.message.message_type).or_default() += 1;
    }
    let bytes =
        zeroize::Zeroizing::new(serde_json::to_vec(&rows).map_err(|_| AppError::internal())?);
    let original = Sha256::digest(bytes.as_slice());
    evidence["observations"] = rows.len().into();
    evidence["types"] = serde_json::to_value(kinds).map_err(|_| AppError::internal())?;
    evidence["gap_count"] = app
        .receive_cursor_local(&account)?
        .map_or(0, |c| c.gaps)
        .into();
    drop(rows);
    drop(bytes);
    drop(app);
    drop(provider);
    drop(repo);
    let reopened = Arc::new(SqliteRepository::open(&db)?);
    let offline = Application::new(reopened.clone(), Arc::new(WindowsWechatProvider));
    let rows = offline.received_texts_local(&account, 0, 100)?;
    let bytes =
        zeroize::Zeroizing::new(serde_json::to_vec(&rows).map_err(|_| AppError::internal())?);
    let matched = Sha256::digest(bytes.as_slice()) == original;
    if !matched {
        return Err(AppError::new(
            "REOPEN_MISMATCH",
            "protected observations differ",
        ));
    }
    drop(bytes);
    drop(rows);
    drop(offline);
    drop(reopened);
    evidence["durable_reopen_matched"] = matched.into();
    evidence["controller_closed"] = true.into();
    evidence["listening"] = false.into();
    evidence["finished_at"] = chrono::Utc::now().to_rfc3339().into();
    for name in ["state.sqlite3", "state.sqlite3-wal", "state.sqlite3-shm"] {
        let f = path.join(name);
        if f.exists() {
            std::fs::remove_file(f).map_err(|_| AppError::internal())?;
        }
    }
    evidence["isolated_state_removed"] = true.into();
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
