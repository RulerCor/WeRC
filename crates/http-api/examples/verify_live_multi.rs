//! Explicit multi-source filehelper read-only test; isolated in-memory store.
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
            .map(|d| {
                Path::new(&source.account_directory)
                    .join(&d.relative_path)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        databases.sort();
        if !(2..=16).contains(&databases.len()) {
            return Err(error("MULTI_SOURCE_SELECTION_REQUIRED"));
        }
        app.receive_once_local(&account)?;
        let gap_before = app.receive_cursor_local(&account)?;
        let first = app.prepare_live_filehelper_multi_with_keys_local(
            &account,
            &databases,
            pid,
            "real-multi-first-0001",
            true,
        )?;
        if first.source_count != databases.len()
            || first.source_snapshot_atomic
            || first.rows.len() != 60
        {
            return Err(error("MULTI_WINDOW_INVALID"));
        }
        let second = app.live_filehelper_multi_page_local(
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
        let back = app.live_filehelper_multi_page_local(
            &account,
            Some((
                second
                    .newer
                    .as_deref()
                    .ok_or_else(|| error("CURSOR_MISSING"))?,
                LiveDirection::Newer,
            )),
            60,
        )?;
        if serde_json::to_vec(&first.rows).map_err(|_| error("SERIALIZE"))?
            != serde_json::to_vec(&back.rows).map_err(|_| error("SERIALIZE"))?
        {
            return Err(error("MULTI_REVERSE_FAILED"));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut contributing = std::collections::BTreeSet::new();
        for row in first.rows.iter().chain(&second.rows) {
            if !seen.insert((row.source.clone(), row.message.local_id.clone())) {
                return Err(error("MULTI_DUPLICATE"));
            }
            contributing.insert(row.source_index);
        }
        let token = second
            .continuation
            .as_deref()
            .ok_or_else(|| error("MULTI_CONTINUATION_MISSING"))?;
        let earlier =
            app.continue_live_filehelper_multi_local(&account, token, "real-multi-next-0001")?;
        for row in &earlier.rows {
            if !seen.insert((row.source.clone(), row.message.local_id.clone())) {
                return Err(error("MULTI_DUPLICATE"));
            }
            contributing.insert(row.source_index);
        }
        let replay_rejected = app
            .continue_live_filehelper_multi_local(&account, token, "real-multi-replay-0001")
            .is_err();
        if !replay_rejected
            || app.receive_cursor_local(&account)? != gap_before
            || !repository.sends(&account)?.is_empty()
            || !app.history_sources_local(&account)?.is_empty()
        {
            return Err(error("MULTI_SIDE_EFFECT_OR_REPLAY"));
        }
        provider.disconnect(None)?;
        let disconnected = app
            .live_filehelper_multi_page_local(&account, None, 60)
            .is_err();
        if !disconnected {
            return Err(error("DISCONNECT_ACCEPTED"));
        }
        Ok(
            serde_json::json!({"passed":true,"application_managed_provider_verified":true,"source_count":databases.len(),"sources_contributing_to_observed_pages":contributing.len(),"first_page_rows":first.rows.len(),"second_page_rows":second.rows.len(),"continuation_rows":earlier.rows.len(),"reverse_page_verified":true,"scoped_id_duplicates":0,"continuation_replay_rejected":true,"disconnected_view_rejected":true,"gap_cursor_unchanged":true,"history_imported":false,"outbox_empty":true,"normal_product_state_opened":false,"product_store":"isolated_in_memory","messages_sent":false,"chat_or_keys_logged":false,"source_snapshot_atomic":false,"source_byte_fingerprints_measured":false,"worker_sha256":worker_hash,"host_sha256":host_hash,"complete_g1a":false}),
        )
    })();
    provider.disconnect(None)?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-live-multi" {
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
