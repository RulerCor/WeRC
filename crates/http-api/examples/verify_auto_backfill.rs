//! Explicit read-only capture/report/export validation in a disposable encrypted in-memory store.
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
    AppError::new(code, "只读补读与归档验证未通过")
}
fn digests(paths: &[PathBuf]) -> AppResult<Vec<String>> {
    let mut out = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| error("SOURCE_PATH"))?;
        for p in [
            path.clone(),
            path.with_file_name(format!("{name}-wal")),
            path.with_file_name(format!("{name}-shm")),
        ] {
            out.push(live_digest::digest(&p).map_err(error)?);
        }
    }
    Ok(out)
}
fn run(
    pid: u32,
    worker: &Path,
    hash: &str,
    host: &Path,
    host_hash: &str,
) -> AppResult<serde_json::Value> {
    let installations = discovery::discover()?.installations;
    let clients: Vec<_> = installations
        .iter()
        .filter(|i| i.process_ids.contains(&pid))
        .collect();
    if clients.len() != 1 || clients[0].version != "4.1.15.13" {
        return Err(error("CLIENT_SELECTION_REQUIRED"));
    }
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
    let fingerprint = hex::encode(Sha256::digest(
        format!("WeRC/setup-source/v1\0{canonical}").as_bytes(),
    ));
    let repo = Arc::new(SqliteRepository::in_memory()?);
    let account = repo
        .setup_account_for_source("isolated automatic recovery verification", &fingerprint)?
        .id;
    let provider = Arc::new(
        ManagedWindowsWechatProvider::with_live_reader(worker.to_owned(), hash.into())
            .with_live_window_limit(120)?,
    );
    let app = Application::new(repo.clone(), provider.clone());
    app.bind_client_local(&account, &clients[0].executable_path)?;
    provider.connect_pinned(&account, host, host_hash, pid, &clients[0].executable_path)?;
    let result = (|| {
        provider.mark_ready(&account)?;

        let paths: Vec<_> = source
            .databases
            .iter()
            .filter(|d| {
                d.wal_present
                    && d.relative_path.starts_with("db_storage/message/message_")
                    && Path::new(&d.relative_path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_prefix("message_"))
                        .and_then(|n| n.strip_suffix(".db"))
                        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            .map(|d| root.join(&d.relative_path))
            .collect();
        if paths.is_empty() {
            return Err(error("SOURCE_UNAVAILABLE"));
        }
        let before = digests(&paths)?;
        for _ in 0..=paths.len() {
            app.discover_backfill_sources_local(&account)?;
        }
        let enrolled = app.backfill_status_local(&account)?;
        if enrolled.source_count != paths.len() {
            return Err(error("DISCOVERY_COUNT_MISMATCH"));
        }
        for _ in 0..3 {
            app.step_backfill_local(&account)?;
        }
        let first = app.backfill_page_local(&account, 0, 1)?;
        if first.data.is_empty() {
            return Err(error("BACKFILL_EMPTY"));
        }
        let stable = first.data[0].rows.first().map(|r| r.id.clone());
        let reopened = Application::new(repo.clone(), provider.clone());
        let resume = reopened.backfill_status_local(&account)?;
        let again = reopened.backfill_page_local(&account, 0, 1)?;
        if again.data[0].rows.first().map(|r| r.id.clone()) != stable {
            return Err(error("RECOVERY_ID_CHANGED"));
        }
        reopened.discover_backfill_sources_local(&account)?;
        let progressed = reopened.step_backfill_local(&account)?;
        if progressed.pages < resume.pages {
            return Err(error("CHECKPOINT_REWOUND"));
        }
        let after = digests(&paths)?;
        Ok(
            serde_json::json!({"passed":before==after,"discovered_sources":enrolled.source_count,"pages_before_reopen":resume.pages,"pages_after_reopen":progressed.pages,"rows_after_reopen":progressed.rows,"stable_id_after_reopen":true,"foreground_view_required":false,"source_db_wal_shm_bytes_unchanged":before==after,"worker_sha256":hash,"host_sha256":host_hash,"normal_product_state_opened":false,"messages_sent":false,"complete_history_scanned":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-auto-backfill" {
        std::process::exit(2);
    }
    let output = Path::new(&args[6]);
    assert!(output.is_absolute() && !output.exists());
    std::fs::create_dir(output).unwrap();
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        Path::new(&args[4]),
        &args[5],
    ) {
        Ok(v) => v,
        Err(e) => serde_json::json!({"passed":false,"error":e.code,"messages_sent":false}),
    };
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("evidence.json"))
        .unwrap();
    f.write_all(&serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
    f.sync_all().unwrap();
    println!("{value}");
    if value["passed"] != true {
        std::process::exit(1);
    }
}
