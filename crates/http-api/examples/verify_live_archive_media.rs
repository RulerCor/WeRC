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
        let mut paths = vec![path.clone(), root.join("db_storage/session/session.db")];
        paths.extend(
            source
                .databases
                .iter()
                .filter(|d| d.relative_path.contains("media"))
                .map(|d| root.join(&d.relative_path)),
        );
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
        let mut media_results = Vec::new();
        let mut attempted = std::collections::HashSet::new();
        for page in 0..capture.pages {
            let saved = reopened.source_capture_page_local(&account, &capture.id, page)?;
            for candidate in reopened.capture_media_page_local(&account, &capture.id, page)? {
                if saved.rows[candidate.row].username != "filehelper" || !candidate.can_fetch {
                    continue;
                }
                let label = match candidate.kind {
                    Some(werc_application::conversation_media::ConversationMediaKind::Image) => {
                        "image"
                    }
                    Some(werc_application::conversation_media::ConversationMediaKind::Voice) => {
                        "voice"
                    }
                    Some(werc_application::conversation_media::ConversationMediaKind::File) => {
                        "file"
                    }
                    _ => continue,
                };
                if !attempted.insert(label) {
                    continue;
                }
                match reopened.archive_capture_media_local(
                    &account,
                    &capture.id,
                    page,
                    candidate.row,
                    "live-archive-media-save",
                ) {
                    Ok(media) => {
                        let again = Application::new(repo.clone(), provider.clone());
                        let mut bytes = zeroize::Zeroizing::new(Vec::new());
                        let check = again.export_capture_media_local(
                            &account,
                            &capture.id,
                            page,
                            candidate.row,
                            "live-archive-media-read",
                            &mut *bytes,
                        )?;
                        if check.sha256 != media.sha256
                            || media.size != bytes.len() as u64
                            || hex::encode(Sha256::digest(&bytes)) != media.sha256
                        {
                            return Err(error("ARCHIVE_MEDIA_MISMATCH"));
                        }
                        media_results.push(serde_json::json!({"kind":label,"passed":true,"size":media.size,"encrypted_reopen":true}));
                    }
                    Err(e) => media_results
                        .push(serde_json::json!({"kind":label,"passed":false,"error":e.code})),
                }
            }
            if attempted.len() == 3 {
                break;
            }
        }
        if media_results.is_empty() {
            return Err(error("NO_MEDIA_SAMPLE"));
        }
        let media_passed = media_results.iter().all(|r| r["passed"] == true);
        let mut bundle_bytes = zeroize::Zeroizing::new(Vec::new());
        let mut bundle = std::io::Cursor::new(&mut *bundle_bytes);
        let bundle_summary = reopened.export_capture_bundle_local(
            &account,
            &capture.id,
            "live-archive-bundle",
            &mut bundle,
        )?;
        if !app.sends_local(&account)?.is_empty() {
            return Err(error("UNEXPECTED_SEND"));
        }
        let after = digests(&paths)?;
        let changed_files:Vec<_>=before.iter().zip(&after).enumerate().filter(|(_, (a,b))|a!=b).map(|(index,_)|serde_json::json!({"source_index":index/3,"component":(["db","wal","shm"][index%3])})).collect();
        let unchanged = changed_files.is_empty();
        Ok(
            serde_json::json!({"passed":media_passed&&unchanged,"media_chain_passed":media_passed,"changed_files":changed_files,"media_results":media_results,"bundle_rows":bundle_summary.rows,"bundle_media":bundle_summary.media,"bundle_missing":bundle_summary.missing_media,"worker_sha256":hash,"host_sha256":host_hash,"capture_rows":capture.rows,"capture_pages":capture.pages,"conversation_count":report.by_conversation.len(),"checkpoint_reopened":true,"report_rows":report.counted_rows,"export_rows":summary.rows,"export_media_written":summary.media_written,"source_db_wal_shm_bytes_unchanged":unchanged,"source_count":paths.len(),"messages_sent":false,"normal_product_state_opened":false,"record_contents_logged":false,"product_store":"isolated_in_memory","native_gaps_closed":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-archive-media" {
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
