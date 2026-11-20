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
        let before = digests(&paths)?;
        let view = app.prepare_live_filehelper_multi_with_keys_local(
            &account,
            &databases,
            pid,
            "multi-conversation-parent-prepare",
            true,
        )?;
        use werc_application::live_conversation::ConversationCommand as C;
        use werc_application::live_view::LiveDirection;
        let revision = repo.key_revision(&account)?;
        let sessions =
            app.live_sessions_local(&account, &view.view_id, None, "live-conversation-sessions")?;
        let mut found = None;
        let mut examined = 0;
        for row in sessions
            .rows
            .iter()
            .filter(|r| r.anchor.username.ends_with("@chatroom"))
            .take(12)
        {
            examined += 1;
            let result = app.live_multi_conversation_local(
                &account,
                C::Open {
                    parent_view_id: view.view_id.clone(),
                    username: row.anchor.username.clone(),
                },
                "live-conversation-open",
            );
            match result {
                Ok(Some(page)) if !page.messages.rows.is_empty() => {
                    found = Some(page);
                    break;
                }
                Ok(_) => {}
                Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => {}
                Err(e) => return Err(e),
            }
        }
        let first = found.ok_or_else(|| error("NO_CONVERSATION_IN_SELECTED_SOURCE"))?;
        let first_count = first.messages.rows.len();
        let resolved = first
            .messages
            .rows
            .iter()
            .filter(|r| r.message.sender_username.is_some())
            .count();
        let distinct = first
            .messages
            .rows
            .iter()
            .filter_map(|r| r.message.sender_username.as_deref())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if resolved == 0 || distinct < 2 {
            return Err(error("GROUP_SENDER_MAPPING_NOT_VERIFIED"));
        }
        let text_count = first
            .messages
            .rows
            .iter()
            .filter(|r| r.message.text.is_some())
            .count();
        let second = first
            .messages
            .older
            .as_ref()
            .map(|cursor| {
                app.live_multi_conversation_local(
                    &account,
                    C::Page {
                        view_id: first.messages.view_id.clone(),
                        cursor: cursor.clone(),
                        direction: LiveDirection::Older,
                    },
                    "live-conversation-page",
                )
            })
            .transpose()?
            .flatten();
        if let Some(second) = &second
            && second.messages.rows.iter().any(|row| {
                first.messages.rows.iter().any(|old| {
                    old.source == row.source && old.message.local_id == row.message.local_id
                })
            })
        {
            return Err(error("CONVERSATION_PAGE_OVERLAP"));
        }
        let wrong_cursor = app
            .live_multi_conversation_local(
                &account,
                C::Page {
                    view_id: first.messages.view_id.clone(),
                    cursor: "invalid".into(),
                    direction: LiveDirection::Older,
                },
                "live-conversation-bad-cursor",
            )
            .is_err();
        let no_recovery = app
            .recover_live_filehelper_multi_local(&account, &first.messages.view_id, true)
            .is_err();
        app.live_multi_conversation_local(
            &account,
            C::Clear {
                view_id: first.messages.view_id.clone(),
            },
            "live-conversation-clear",
        )?;
        let cleared = app
            .live_multi_conversation_local(
                &account,
                C::Page {
                    view_id: first.messages.view_id.clone(),
                    cursor: first.messages.older.clone().unwrap_or_default(),
                    direction: LiveDirection::Older,
                },
                "live-conversation-after-clear",
            )
            .is_err();
        if !wrong_cursor
            || !no_recovery
            || !cleared
            || first.sending_enabled
            || repo.key_revision(&account)? != revision
            || !app.sends_local(&account)?.is_empty()
        {
            return Err(error("UNEXPECTED_STATE_CHANGE"));
        }
        let unchanged = before == digests(&paths)?;
        if !unchanged {
            return Err(error("SOURCE_CHANGED_DURING_TEST"));
        }
        Ok(
            serde_json::json!({"passed":true,"worker_sha256":hash,"host_sha256":host_hash,
            "candidate_conversations_examined":examined,"group_conversation":true,"sender_rows":resolved,"sender_distinct":distinct,"sender_unknown_rows":first_count-resolved,"non_filehelper_multi_conversation_verified":true,
            "source_count":databases.len(),"contributing_sources_in_first_page":first.messages.rows.iter().map(|r|r.source_index).collect::<std::collections::BTreeSet<_>>().len(),"multi_session_cache_verified":true,"first_page_rows":first_count,"first_page_text_rows":text_count,"second_page_rows":second.as_ref().map(|p|p.messages.rows.len()),
            "wrong_cursor_and_filehelper_recovery_rejected":true,"cleared_view_rejected":true,
            "temporary_session_keys_not_persisted":true,"source_db_wal_shm_bytes_unchanged":true,"observed_database_count":paths.len(),
            "outbox_empty":true,"messages_sent":false,"normal_product_state_opened":false,"conversation_records_logged":false,
            "product_store":"isolated_in_memory","complete_conversation_history":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-multi-conversation" {
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
