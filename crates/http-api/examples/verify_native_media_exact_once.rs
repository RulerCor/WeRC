//! One explicitly authorized automatic test in a disposable encrypted product store.
//! Never retry or run automatically: an existing output directory prevents submission.
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
    AppError::new(code, "媒体精确回执验证未通过")
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
    output: &Path,
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
    let repo = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let account = repo
        .setup_account_for_source("isolated exact media identity verification", &fingerprint)?
        .id;
    let provider = Arc::new(
        ManagedWindowsWechatProvider::with_live_reader(worker.to_owned(), hash.into())
            .with_live_window_limit(120)?,
    );
    let mut app = Application::new(repo.clone(), provider.clone());
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
        let paths = vec![path.clone(), root.join("db_storage/general/general.db")];

        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;

        app.prepare_live_filehelper_local(&account, text, "exact-receipt-source", true)?;
        let body = "自动化测试：WeRC 附件精确回执与重连核验 2026-11-13（仅一次）";
        let asset = app.upload_file_local(
            &account,
            "WeRC-精确媒体回执自动化测试.txt",
            body.as_bytes().to_vec(),
        )?;
        let draft = app.create_file_send_local(
            &account,
            "filehelper",
            &asset.id,
            "native-media-exact-20261113-once",
        )?;
        app.approve_text_send_local(
            &account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
        )?;
        let mut guard = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join("submission-started.json"))
            .map_err(|_| error("ALREADY_ATTEMPTED"))?;
        guard.write_all(serde_json::json!({"outbox_id":draft.metadata.id,"target":"filehelper","test_text":body,"started_at":chrono::Utc::now().to_rfc3339()}).to_string().as_bytes()).map_err(|_|error("EVIDENCE_WRITE"))?;
        guard.sync_all().map_err(|_| error("EVIDENCE_WRITE"))?;
        let sent = app.execute_send_local(&account, &draft.metadata.id)?;
        std::fs::write(
            output.join("submitted.json"),
            serde_json::to_vec_pretty(&sent).unwrap(),
        )
        .map_err(|_| error("EVIDENCE_WRITE"))?;
        let mut state = sent.state;
        let mut query_errors = Vec::new();
        for _ in 0..45 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            match app.reconcile_send_local(&account, &draft.metadata.id) {
                Ok(status) => {
                    state = status.state;
                    if state == werc_domain::outbox::SendState::ServerAccepted {
                        break;
                    }
                }
                Err(e) => {
                    if !query_errors.contains(&e.code) {
                        query_errors.push(e.code);
                    }
                }
            }
        }
        let fields = match werc_application::ports::CapabilityProvider::resolve_send_context(
            &*provider,
            &account,
            "filehelper",
        ) {
            Ok(context) => provider
                .observed_text_task_fields(&context, sent.attempt_id.as_deref().unwrap_or(""))
                .ok(),
            Err(_) => None,
        };
        std::fs::write(
            output.join("task-fields.json"),
            serde_json::to_vec_pretty(&fields).unwrap(),
        )
        .map_err(|_| error("EVIDENCE_WRITE"))?;
        let proof = repo.library_get(&account, &format!("send-exact-{}", draft.metadata.id))?;
        let proof_persisted = proof.header.kind == "send_exact";
        let before = digests(&paths)?;
        provider.disconnect(None)?;
        app.clear_live_filehelper_local()?;
        // Reopen the encrypted repository and establish a new native epoch. The
        // subsequent query has no access to the old bridge's receipt slots.
        let reopened = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
        app = Application::new(reopened.clone(), provider.clone());
        provider.connect_pinned(&account, host, host_hash, pid, &clients[0].executable_path)?;
        provider.mark_ready(&account)?;
        let after = app.reconcile_send_local(&account, &draft.metadata.id)?;
        let cannot_resubmit = app
            .execute_send_local(&account, &draft.metadata.id)
            .is_err();
        let unchanged = before == digests(&paths)?;
        let passed = state == werc_domain::outbox::SendState::ServerAccepted
            && after.state == state
            && cannot_resubmit
            && unchanged
            && proof_persisted;
        Ok(
            serde_json::json!({"passed":passed,"before_reconnect_state":state,"after_reconnect_state":after.state,"proof_persisted":proof_persisted,"store_reopened":true,"cannot_resubmit":cannot_resubmit,"query_errors":query_errors,"source_db_wal_shm_unchanged_during_reconnect_read":unchanged,"messages_sent":1,"recipient_confirmed":false,"normal_product_state_opened":false,"worker_sha256":hash,"host_sha256":host_hash,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-one-media-exact-receipt-test" {
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
        output,
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"send_attempted":output.join("submission-started.json").exists(),"do_not_resend":true})
        }
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
