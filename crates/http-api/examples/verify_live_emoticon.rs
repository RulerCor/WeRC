//! Explicit read-only validation of existing test emoticons; isolated encrypted store.
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
    AppError::new(code, "只读测试表情验证未通过")
}
fn digest_sources(paths: &[PathBuf]) -> AppResult<Vec<String>> {
    let mut output = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| error("UNSAFE_PATH"))?;
        for file in [
            path.clone(),
            path.with_file_name(format!("{name}-wal")),
            path.with_file_name(format!("{name}-shm")),
        ] {
            output.push(live_digest::digest(&file).map_err(error)?);
        }
    }
    Ok(output)
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
    let root = Path::new(&source.account_directory);
    let canonical = root
        .canonicalize()
        .map_err(|_| error("SOURCE_UNAVAILABLE"))?
        .to_string_lossy()
        .to_lowercase();
    let fp = hex::encode(Sha256::digest(
        format!("WeRC/setup-source/v1\0{canonical}").as_bytes(),
    ));
    let repository = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let account = repository
        .setup_account_for_source("isolated emoticon verification", &fp)?
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
        app.receive_once_local(&account)?;
        let gap = app.receive_cursor_local(&account)?;
        let mut databases: Vec<_> = source
            .databases
            .iter()
            .filter(|d| d.wal_present && named(Path::new(&d.relative_path), "message_"))
            .collect();
        if databases.len() > 16 {
            return Err(error("SOURCE_LIMIT"));
        }
        databases.sort_by(|a, b| b.relative_path.cmp(&a.relative_path));
        for database in databases {
            let path = root.join(&database.relative_path);
            let text = path.to_str().ok_or_else(|| error("UNSAFE_PATH"))?;
            app.bind_live_source_local(&account, text, true)?;
            app.acquire_database_key_local(&account, pid, text, true)?;
            let page = match app.prepare_live_filehelper_local(
                &account,
                text,
                "real-emoticon-window-0001",
                true,
            ) {
                Ok(p) => p,
                Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => continue,
                Err(e) => return Err(e),
            };
            let paths = vec![path.clone()];
            let before = digest_sources(&paths)?;
            let key_revision = repository.key_revision(&account)?;
            let mut saved = Vec::new();
            for row in page
                .rows
                .iter()
                .filter(|r| {
                    r.message_type.parse::<i64>().unwrap_or(0) & 0xffffffff == 47
                        && r.metadata
                            .as_ref()
                            .is_some_and(|m| m.contains("df4813390984d6155aabb73ea22dade0"))
                })
                .take(8)
            {
                let asset = match app.save_live_emoticon_local(
                    &account,
                    &page.view_id,
                    &row.local_id,
                    "real-emoticon-save-0001",
                ) {
                    Ok(asset) => asset,
                    Err(e) if e.code == "EMOTICON_CACHE_UNAVAILABLE" => continue,
                    Err(e) => return Err(e),
                };
                let restored = app.emoticon_contents_local(&account, &asset.id)?;
                if restored.bytes.as_slice()
                    != include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif")
                {
                    return Err(error("KNOWN_TEST_CONTENT_MISMATCH"));
                }
                saved.push(asset);
            }
            if saved.is_empty() {
                continue;
            }
            if repository.key_revision(&account)? != key_revision {
                return Err(error("KEY_REVISION_CHANGED"));
            }
            if app.receive_cursor_local(&account)? != gap || !app.sends_local(&account)?.is_empty()
            {
                return Err(error("UNEXPECTED_STATE_CHANGE"));
            }
            if before != digest_sources(&paths)? {
                return Err(error("SOURCE_BYTES_CHANGED"));
            }
            return Ok((saved, gap, true, paths.len()));
        }
        Err(error("TEST_EMOTICON_NOT_FOUND"))
    })();
    provider.disconnect(None)?;
    let (saved, gap, unchanged, source_count) = result?;
    app.clear_live_filehelper_local()?;
    drop(app);
    drop(repository);
    drop(provider);
    let repo = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let offline = Application::new(
        repo.clone(),
        Arc::new(werc_wechat_win::WindowsWechatProvider),
    );
    let mut reports = Vec::new();
    for asset in saved {
        let gif = offline.emoticon_contents_local(&account, &asset.id)?;
        if hex::encode(Sha256::digest(gif.bytes.as_slice())) != asset.sha256 {
            return Err(error("REOPEN_MISMATCH"));
        }
        reports.push(serde_json::json!({"gif_bytes":asset.size,"gif_sha256":asset.sha256,"full_gif_equal":true}));
    }
    if offline.receive_cursor_local(&account)? != gap || !offline.sends_local(&account)?.is_empty()
    {
        return Err(error("UNEXPECTED_STATE_CHANGE"));
    }
    drop(offline);
    drop(repo);
    Ok(
        serde_json::json!({"passed":true,"worker_sha256":worker_hash,"host_sha256":host_hash,"emoticons":reports,"source_count":source_count,"source_db_wal_shm_bytes_unchanged":unchanged,"encrypted_store_closed_and_reopened":true,"temporary_media_keys_not_persisted":true,"existing_outgoing_type47_only":true,"incoming_hook_emoticon_verified":false,"gap_cursor_unchanged":true,"outbox_empty":true,"messages_sent":false,"normal_product_state_opened":false,"receive_hook_emoticon_verified":false,"full_product_completed":false}),
    )
}
fn named(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix(prefix))
        .and_then(|s| s.strip_suffix(".db"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-existing-test-emoticons" {
        std::process::exit(2);
    }
    let output = PathBuf::from(&args[6]);
    assert!(output.is_absolute() && !output.exists());
    std::fs::create_dir(&output).unwrap();
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        Path::new(&args[4]),
        &args[5],
        &output,
    ) {
        Ok(v) => v,
        Err(e) => serde_json::json!({"passed":false,"error":e.code,"messages_sent":false}),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("evidence.json"))
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    println!("{value}");
    if value["passed"] != true {
        std::process::exit(1);
    }
}
