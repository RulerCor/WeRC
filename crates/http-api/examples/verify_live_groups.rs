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
        let view = app.prepare_live_filehelper_multi_with_keys_local(
            &account,
            &databases,
            pid,
            "multi-conversation-parent-prepare",
            true,
        )?;

        let revision = repo.key_revision(&account)?;
        let sessions =
            app.live_sessions_local(&account, &view.view_id, None, "owned-group-real-sessions")?;
        let mut reports = Vec::new();
        for row in sessions
            .rows
            .iter()
            .filter(|r| r.anchor.username.ends_with("@chatroom"))
            .take(1)
        {
            let first = match app.live_group_members_local(
                &account,
                &view.view_id,
                &row.anchor.username,
                None,
                "owned-group-real-first",
            ) {
                Ok(page) => page,
                Err(e) if e.code == "LIVE_GROUP_NOT_IN_CACHE" => continue,
                Err(e) => return Err(e),
            };
            let second = first
                .next
                .as_ref()
                .map(|cursor| {
                    app.live_group_members_local(
                        &account,
                        &view.view_id,
                        &row.anchor.username,
                        Some(cursor),
                        "owned-group-real-second",
                    )
                })
                .transpose()?;
            if second.as_ref().is_some_and(|p| {
                p.rows
                    .first()
                    .is_some_and(|r| r.anchor.key() <= first.rows.last().unwrap().anchor.key())
            }) {
                return Err(error("GROUP_PAGE_ORDER"));
            }
            if app
                .live_group_members_local(
                    &account,
                    "invalid-view",
                    &row.anchor.username,
                    None,
                    "owned-group-wrong-view",
                )
                .is_ok()
                || app
                    .live_group_members_local(
                        &account,
                        &view.view_id,
                        &row.anchor.username,
                        Some("invalid"),
                        "owned-group-wrong-cursor",
                    )
                    .is_ok()
            {
                return Err(error("GROUP_BINDING_FAILED"));
            }
            let mut matched_pins=0usize;
            if let Some(pins)=&first.cached_pins {
                use werc_application::live_conversation::ConversationCommand as Command;
                let mut conversation=app.live_multi_conversation_local(&account,Command::Open{parent_view_id:view.view_id.clone(),username:row.anchor.username.clone()},"owned-group-pin-open")?.ok_or_else(||error("GROUP_PIN_VIEW_MISSING"))?;
                for pin in pins {
                    let signed=(pin.server_id.parse::<u64>().map_err(|_|error("GROUP_PIN_INVALID_ID"))? as i64).to_string();
                    let mut query=werc_domain::live::LiveMessageQuery::default();query.server_id=Some(signed.clone());
                    conversation=app.live_multi_conversation_local(&account,Command::Find{view_id:conversation.messages.view_id.clone(),query},"group-pin-exact-find")?.ok_or_else(||error("GROUP_PIN_FIND_MISSING"))?;
                    if conversation.messages.rows.iter().any(|r|r.message.server_id.as_deref()!=Some(signed.as_str())) {return Err(error("GROUP_PIN_IDENTITY_MISMATCH"));}
                    matched_pins+=usize::from(!conversation.messages.rows.is_empty());
                }
                app.live_multi_conversation_local(&account,Command::Clear{view_id:conversation.messages.view_id.clone()},"owned-group-pin-clear")?;
            }
            reports.push(serde_json::json!({"cached_pins_known":first.cached_pins.is_some(),"cached_pins":first.cached_pins.as_ref().map(|p|p.len()),"pins_matched_in_selected_sources":matched_pins,"first_rows":first.rows.len(),"second_rows":second.as_ref().map(|p|p.rows.len()),"resolved":first.rows.iter().filter(|r|r.anchor.username.is_some()).count(),"with_display_name":first.rows.iter().filter(|r|r.nickname.is_some()||r.remark.is_some()).count(),"group_nicknames":first.rows.iter().filter(|r|r.group_nickname.is_some()).count(),"local_owner_recorded":first.owner_username.is_some(),"complete_server_membership":first.complete_server_membership}));
            if reports.len() == 1 {
                break;
            }
        }
        if reports.is_empty()
            || repo.key_revision(&account)? != revision
            || !app.sends_local(&account)?.is_empty()
        {
            return Err(error("GROUP_VALIDATION_INCOMPLETE"));
        }
        if before != digests(&paths)? {
            return Err(error("SOURCE_CHANGED_DURING_TEST"));
        }
        Ok(
            serde_json::json!({"passed":true,"worker_sha256":hash,"host_sha256":host_hash,"groups":reports,"multi_parent_verified":true,"source_count":paths.len(),"source_db_wal_shm_bytes_unchanged":true,"temporary_keys_not_persisted":true,"wrong_view_and_cursor_rejected":true,"outbox_empty":true,"messages_sent":false,"normal_product_state_opened":false,"group_records_logged":false,"complete_server_membership":false,"product_store":"isolated_in_memory","complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-groups" {
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
        Err(e) => serde_json::json!({"passed":false,"error":e.code,"message":e.message,"messages_sent":false}),
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
