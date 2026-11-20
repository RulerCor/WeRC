//! Explicit real read-only Application/ManagedProvider test; isolated in-memory store.
#[path = "../../wechat-win/examples/support/live_digest.rs"]
mod live_digest;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use werc_application::{Application, live_view::LiveDirection, ports::Repository};
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
    let repository = Arc::new(SqliteRepository::in_memory()?);
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
            let second = app.live_filehelper_page_local(
                &account,
                Some((
                    first
                        .older
                        .as_deref()
                        .ok_or_else(|| error("WINDOW_TOO_SMALL"))?,
                    LiveDirection::Older,
                )),
                60,
            )?;
            let back = app.live_filehelper_page_local(
                &account,
                Some((
                    second
                        .newer
                        .as_deref()
                        .ok_or_else(|| error("VIEW_CURSOR_MISSING"))?,
                    LiveDirection::Newer,
                )),
                60,
            )?;
            if first.rows.iter().map(|r| &r.local_id).collect::<Vec<_>>()
                != back.rows.iter().map(|r| &r.local_id).collect::<Vec<_>>()
                || first
                    .rows
                    .iter()
                    .any(|r| second.rows.iter().any(|s| r.local_id == s.local_id))
            {
                return Err(error("LIVE_PAGE_MISMATCH"));
            }
            let continuation = second
                .continuation
                .as_deref()
                .ok_or_else(|| error("SOURCE_WINDOW_TOO_SMALL"))?;
            let earlier = app.continue_live_filehelper_local(
                &account,
                continuation,
                "real-live-continue-0001",
            )?;
            let boundary = second
                .rows
                .last()
                .ok_or_else(|| error("WINDOW_TOO_SMALL"))?;
            let boundary_key = (
                boundary.timestamp,
                boundary
                    .local_id
                    .parse::<i64>()
                    .map_err(|_| error("LIVE_PAGE_MISMATCH"))?,
            );
            if earlier.rows.is_empty()
                || earlier.rows.iter().any(|r| {
                    (r.timestamp, r.local_id.parse::<i64>().unwrap_or(i64::MAX)) >= boundary_key
                })
            {
                return Err(error("LIVE_CONTINUATION_MISMATCH"));
            }
            let continuation_replay_rejected = app
                .continue_live_filehelper_local(&account, continuation, "real-live-continue-replay")
                .is_err();
            if !continuation_replay_rejected {
                return Err(error("CONTINUATION_REPLAY_ACCEPTED"));
            }
            if !app.history_sources_local(&account)?.is_empty()
                || !repository.sends(&account)?.is_empty()
                || repository.receive_cursor(&account)?.is_some()
            {
                return Err(error("UNEXPECTED_PRODUCT_WRITE"));
            }
            let after = digests(&path)?;
            let compressed_text_rows = first
                .rows
                .iter()
                .chain(&second.rows)
                .chain(&earlier.rows)
                .filter(|r| r.content_status == "text_zstd")
                .count();
            provider.disconnect(None)?;
            let disconnected_rejected = app.live_filehelper_page_local(&account, None, 60).is_err();
            return Ok(
                serde_json::json!({"passed":true,"worker_sha256":worker_hash,"host_sha256":host_hash,"application_managed_provider_verified":true,"source_account_binding_verified":true,"native_login_directory_binding_verified":true,"source_db_wal_shm_bytes_unchanged":before==after,"first_page_rows":first.rows.len(),"second_page_rows":second.rows.len(),"source_window_limit":120,"continuation_rows":earlier.rows.len(),"source_continuation_verified":true,"compressed_text_rows":compressed_text_rows,"continuation_replay_rejected":continuation_replay_rejected,"bidirectional_pages_verified":true,"disconnected_view_rejected":disconnected_rejected,"normal_product_state_opened":false,"product_store":"isolated_in_memory","raw_keys_or_chat_logged":false,"messages_sent":false,"history_imported":false,"gaps_cleared":false,"full_g1a_verified":false}),
            );
        }
        Err(error("LIVE_CONVERSATION_NOT_IN_SOURCE"))
    })();
    provider.disconnect(None)?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-live-application" {
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
