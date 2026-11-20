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
        .setup_account_for_source("isolated capture verification", &fingerprint)?
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
        let candidate = source
            .databases
            .iter()
            .filter(|d| {
                d.wal_present
                    && d.relative_path
                        .strip_prefix("db_storage/message/message_")
                        .and_then(|s| s.strip_suffix(".db"))
                        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            })
            .max_by_key(|d| &d.relative_path)
            .ok_or_else(|| error("SOURCE_UNAVAILABLE"))?;
        let path = root.join(&candidate.relative_path);
        let paths = vec![path.clone(), root.join("db_storage/session/session.db")];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, text, "live-sessions-owned-view", true)?;
        let newest = view
            .rows
            .iter()
            .map(|r| r.timestamp)
            .max()
            .ok_or_else(|| error("NO_SAMPLE"))?;
        let from = newest.saturating_sub(86400);
        let until = newest + 1;
        let mut capture =
            app.start_source_capture_local(&account, &view.view_id, Some(from), Some(until))?;
        let mut iterations = 0;
        while !capture.complete_selected_sources && iterations < 200 {
            capture =
                app.step_source_capture_local(&account, &capture.id, "live-capture-step-0001")?;
            iterations += 1;
        }
        if !capture.complete_selected_sources || capture.rows == 0 {
            return Err(error("CAPTURE_INCOMPLETE"));
        }
        let reopened = Application::new(repo.clone(), provider.clone());
        let recovered = reopened.source_capture_status_local(&account, &capture.id)?;
        if recovered.pages != capture.pages || recovered.rows != capture.rows {
            return Err(error("CHECKPOINT_MISMATCH"));
        }
        let report = reopened.capture_report_local(
            &account,
            &capture.id,
            &werc_application::capture_tools::CaptureFilter::default(),
            "live-capture-report-0001",
        )?;
        if report.counted_rows != capture.rows
            || report.sent + report.received + report.unknown_sender != capture.rows
        {
            return Err(error("REPORT_MISMATCH"));
        }
        let mut output = zeroize::Zeroizing::new(Vec::new());
        let summary = reopened.export_capture_local(
            &account,
            &capture.id,
            "jsonl",
            &werc_application::capture_tools::CaptureExportOptions::default(),
            "live-capture-export-0001",
            &mut *output,
        )?;
        let lines = output
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .count() as u64;
        if summary.rows != capture.rows || lines != capture.rows {
            return Err(error("EXPORT_MISMATCH"));
        }
        for line in output.split(|b| *b == b'\n').filter(|s| !s.is_empty()) {
            let row: serde_json::Value =
                serde_json::from_slice(line).map_err(|_| error("EXPORT_JSON_INVALID"))?;
            if row.get("metadata").is_some_and(|v| !v.is_null())
                || row
                    .get("message")
                    .and_then(|v| v.get("metadata"))
                    .is_some_and(|v| !v.is_null())
            {
                return Err(error("PRIVATE_METADATA_EXPORTED"));
            }
        }
        if !app.sends_local(&account)?.is_empty() {
            return Err(error("UNEXPECTED_SEND"));
        }
        let unchanged = before == digests(&paths)?;
        if !unchanged {
            return Err(error("SOURCE_CHANGED_DURING_TEST"));
        }
        Ok(
            serde_json::json!({"passed":true,"worker_sha256":hash,"host_sha256":host_hash,"capture_rows":capture.rows,"capture_pages":capture.pages,"conversation_count":report.by_conversation.len(),"checkpoint_reopened":true,"report_rows":report.counted_rows,"export_rows":summary.rows,"export_media_written":summary.media_written,"source_db_wal_shm_bytes_unchanged":true,"source_count":paths.len(),"messages_sent":false,"normal_product_state_opened":false,"record_contents_logged":false,"product_store":"isolated_in_memory","native_gaps_closed":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-capture" {
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
