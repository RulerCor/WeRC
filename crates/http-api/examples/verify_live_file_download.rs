//! Explicit real read-only Application/ManagedProvider test; isolated in-memory store.
#[path = "../../wechat-win/examples/support/live_digest.rs"]
mod live_digest;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{data_sources, discovery, managed_messages::ManagedWindowsWechatProvider};
fn error(code: &str) -> AppError {
    AppError::new(code, "当前来源只读装配验证未通过")
}
fn digests(path: &Path) -> AppResult<Vec<String>> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| error("UNSAFE_PATH"))?;
    [
        path.to_path_buf(),
        path.with_file_name(format!("{name}-wal")),
        path.with_file_name(format!("{name}-shm")),
    ]
    .iter()
    .map(|p| live_digest::digest(p).map_err(error))
    .collect()
}
fn run(
    pid: u32,
    worker: &Path,
    worker_hash: &str,
    host: &Path,
    host_hash: &str,
    output: &Path,
) -> AppResult<serde_json::Value> {
    let installations = discovery::discover()?.installations;
    let candidates: Vec<_> = installations
        .iter()
        .filter(|i| i.process_ids.contains(&pid))
        .collect();
    if candidates.len() != 1 || candidates[0].version != "4.1.15.13" {
        return Err(error("CLIENT_SELECTION_REQUIRED"));
    }
    let installation = candidates[0];
    let catalog = data_sources::discover()?;
    if catalog.sources.len() != 1 {
        return Err(error("SOURCE_SELECTION_REQUIRED"));
    }
    let source = &catalog.sources[0];
    let canonical = Path::new(&source.account_directory)
        .canonicalize()
        .map_err(|_| error("SOURCE_UNAVAILABLE"))?
        .to_string_lossy()
        .to_lowercase();
    let fp = hex::encode(Sha256::digest(
        format!("WeRC/setup-source/v1\0{canonical}").as_bytes(),
    ));
    let repository = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let account = repository
        .setup_account_for_source("isolated live validation", &fp)?
        .id;
    let provider = Arc::new(
        ManagedWindowsWechatProvider::with_live_reader(worker.to_path_buf(), worker_hash.into())
            .with_live_window_limit(120)?,
    );
    let app = Application::new(repository.clone(), provider.clone());
    app.bind_client_local(&account, &installation.executable_path)?;
    provider.connect_pinned(
        &account,
        host,
        host_hash,
        pid,
        &installation.executable_path,
    )?;
    provider.mark_ready(&account)?;
    let result = (|| {
        let mut databases: Vec<_> = source
            .databases
            .iter()
            .filter(|d| {
                let name = Path::new(&d.relative_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                name.strip_prefix("message_")
                    .and_then(|n| n.strip_suffix(".db"))
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    && d.wal_present
            })
            .collect();
        if databases.len() > 16 {
            return Err(error("SOURCE_LIMIT"));
        }
        databases.sort_by(|a, b| b.relative_path.cmp(&a.relative_path));
        for database in databases {
            let path = Path::new(&source.account_directory).join(&database.relative_path);
            let path_text = path.to_str().ok_or_else(|| error("UNSAFE_PATH"))?;
            let before = digests(&path)?;
            app.bind_live_source_local(&account, path_text, true)?;
            app.acquire_database_key_local(&account, pid, path_text, true)?;
            let first = match app.prepare_live_filehelper_local(
                &account,
                path_text,
                "real-live-application-0001",
                true,
            ) {
                Ok(p) => p,
                Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => continue,
                Err(e) => return Err(e),
            };
            for row in &first.rows {
                if row.message_type.parse::<i64>().unwrap_or(0) & 0xffffffff != 49 {
                    continue;
                }
                let Some(metadata) = &row.metadata else {
                    continue;
                };
                let message = werc_domain::receive::ReceivedText {
                    sequence: 1,
                    native_message_id: "1".into(),
                    text: String::new(),
                    message_type: 49,
                    metadata: Some(metadata.clone()),
                };
                let Ok(descriptor) = werc_domain::received_media::file_descriptor(&message) else {
                    continue;
                };
                if *descriptor.file_name != "WeRC-原生文件验证.txt"
                    || descriptor.media.declared_size != 73
                {
                    continue;
                }
                let content =
                    app.download_live_file_local(&account, &first.view_id, &row.local_id)?;
                let bytes = content.bytes.clone();
                let sha = hex::encode(Sha256::digest(&bytes));
                if sha != "d412e8d5e0dd477462f3f113d0acc6f142cbe1c7c4d73a6be39097564bea4563" {
                    return Err(error("KNOWN_TEST_CONTENT_MISMATCH"));
                }
                let asset =
                    app.upload_file_local(&account, &descriptor.file_name, bytes.to_vec())?;
                provider.disconnect(None)?;
                let offline = Application::new(
                    Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?),
                    Arc::new(werc_wechat_win::WindowsWechatProvider),
                );
                let principal = werc_domain::Principal {
                    id: "owned-local-verification".into(),
                    account_ids: vec![account.clone()],
                    scopes: vec!["media:read".into()],
                };
                let restored = offline.download_file(&principal, &account, &asset.id)?;
                if restored.bytes.as_slice() != bytes.as_slice() {
                    return Err(error("OFFLINE_MISMATCH"));
                }
                return Ok(
                    serde_json::json!({"passed":true,"worker_sha256":worker_hash,"host_sha256":host_hash,"native_file_download_verified":true,"bytes":bytes.len(),"sha256":sha,"encrypted_store_reopened":true,"separate_reader_after_disconnect":true,"source_db_wal_shm_bytes_unchanged":before==digests(&path)?,"normal_product_state_opened":false,"messages_sent":false,"outbox_empty":app.sends_local(&account)?.is_empty(),"full_product_completed":false,"receive_hook_file_observation_verified":false,"live_application_download_verified":true}),
                );
            }
        }
        Err(error("LIVE_CONVERSATION_NOT_IN_SOURCE"))
    })();
    provider.disconnect(None)?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-test-file-download" {
        std::process::exit(2);
    }
    let pid: u32 = args[1].parse().expect("explicit PID");
    let out = PathBuf::from(&args[6]);
    assert!(out.is_absolute() && !out.exists());
    std::fs::create_dir(&out).unwrap();
    let result = match run(
        pid,
        Path::new(&args[2]),
        &args[3],
        Path::new(&args[4]),
        &args[5],
        &out,
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"messages_sent":false,"full_g1a_verified":false})
        }
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out.join("evidence.json"))
        .unwrap();
    file.write_all(serde_json::to_string_pretty(&result).unwrap().as_bytes())
        .unwrap();
    file.sync_all().unwrap();
    println!("{result}");
    if result["passed"] != true {
        std::process::exit(1);
    }
}
