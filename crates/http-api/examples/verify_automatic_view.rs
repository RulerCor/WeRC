//! Explicit read-only selected multi-source conversation validation. Only aggregate evidence leaves memory.
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
    AppError::new(code, "只读会话消息验证未通过")
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
        .setup_account_for_source("isolated sessions verification", &fingerprint)?
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
        let candidates: Vec<_> = source
            .databases
            .iter()
            .filter(|d| {
                d.wal_present
                    && d.relative_path
                        .strip_prefix("db_storage/message/message_")
                        .and_then(|s| s.strip_suffix(".db"))
                        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            })
            .collect();
        if !(2..=16).contains(&candidates.len()) {
            return Err(error("MULTI_SOURCE_SELECTION_REQUIRED"));
        }
        let databases: Vec<String> = candidates
            .iter()
            .map(|d| root.join(&d.relative_path).to_string_lossy().into_owned())
            .collect();
        let mut paths: Vec<_> = databases.iter().map(PathBuf::from).collect();
        paths.push(root.join("db_storage/session/session.db"));
        paths.push(root.join("db_storage/contact/contact.db"));
        let before = digests(&paths)?;
        let view = app.prepare_automatic_live_local(&account, "readonly-auto-view", false)?;
        let werc_application::backfill::AutomaticLivePage::Multiple(view) = view else {
            return Err(error("AUTOMATIC_MERGE_EXPECTED"));
        };
        if view.source_count != databases.len() {
            return Err(error("AUTOMATIC_SOURCE_COUNT"));
        }
        let profile =
            app.current_profile_local(&account, &view.view_id, "readonly-current-profile")?;
        if profile.username.is_empty() {
            return Err(error("CURRENT_PROFILE_EMPTY"));
        }
        let revision = repo.key_revision(&account)?;
        let sessions =
            app.live_sessions_local(&account, &view.view_id, None, "owned-group-real-sessions")?;
        if sessions.rows.is_empty() {
            return Err(error("SESSIONS_EMPTY"));
        }
        let single = app.prepare_automatic_live_local(&account, "readonly-auto-tools", true)?;
        if !matches!(
            single,
            werc_application::backfill::AutomaticLivePage::Single(_)
        ) {
            return Err(error("SINGLE_VIEW_EXPECTED"));
        }
        if !app.sends_local(&account)?.is_empty() {
            return Err(error("OUTBOX_NOT_EMPTY"));
        }
        if before != digests(&paths)? {
            return Err(error("SOURCE_CHANGED_DURING_TEST"));
        }
        Ok(
            serde_json::json!({"passed":true,"worker_sha256":hash,"host_sha256":host_hash,"automatic_source_count":view.source_count,"sessions":sessions.rows.len(),"profile_login_present":!profile.username.is_empty(),"profile_nickname_present":profile.nickname.is_some(),"profile_avatar_available":profile.avatar_available,"single_tools_view":true,"key_revision":revision,"source_db_wal_shm_bytes_unchanged":true,"outbox_empty":true,"messages_sent":false,"normal_product_state_opened":false,"product_store":"isolated_in_memory","complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-automatic-view" {
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
