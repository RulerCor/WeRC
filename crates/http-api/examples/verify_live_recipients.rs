//! Explicit authenticated recipient preparation validation; no messages submitted. Only aggregate evidence leaves memory.
#[path = "../../wechat-win/examples/support/live_digest.rs"]
mod live_digest;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
    recipients::RecipientSelection,
};
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
            .filter(|r| r.anchor.username != "filehelper")
            .take(12)
        {
            examined += 1;
            let result = app.live_conversation_local(
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
        let prepared = app.prepare_recipient_local(
            &account,
            RecipientSelection {
                view_id: first.messages.view_id.clone(),
                multi: false,
            },
        )?;
        let selected = provider.resolve_send_context(&account, &prepared.target_id)?;
        if selected.target_id != first.username
            || selected.sender_id == selected.target_id
            || prepared.expires_at.is_none()
        {
            return Err(error("RECIPIENT_SELECTION_MISMATCH"));
        }
        if provider
            .resolve_send_context(&account, "werc_nonexistent_automation_target")
            .is_ok()
        {
            return Err(error("UNVERIFIED_TARGET_ACCEPTED"));
        }
        let first_count = first.messages.rows.len();
        let text_count = first
            .messages
            .rows
            .iter()
            .filter(|r| r.text.is_some())
            .count();
        let second = first
            .messages
            .older
            .as_ref()
            .map(|cursor| {
                app.live_conversation_local(
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
                first
                    .messages
                    .rows
                    .iter()
                    .any(|old| old.local_id == row.local_id)
            })
        {
            return Err(error("CONVERSATION_PAGE_OVERLAP"));
        }
        let wrong_cursor = app
            .live_conversation_local(
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
            .recover_live_filehelper_local(&account, &first.messages.view_id, true)
            .is_err();
        app.live_conversation_local(
            &account,
            C::Clear {
                view_id: first.messages.view_id.clone(),
            },
            "live-conversation-clear",
        )?;
        let cleared = app
            .live_conversation_local(
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
        if provider.resolve_send_context(&account, &prepared.target_id)? != selected
            || app
                .prepare_recipient_local(
                    &account,
                    RecipientSelection {
                        view_id: first.messages.view_id.clone(),
                        multi: false,
                    },
                )
                .is_ok()
        {
            return Err(error("RECIPIENT_VIEW_LIFETIME_MISMATCH"));
        }
        let unchanged = before == digests(&paths)?;
        if !unchanged {
            return Err(error("SOURCE_CHANGED_DURING_TEST"));
        }
        Ok(
            serde_json::json!({"passed":true,"worker_sha256":hash,"host_sha256":host_hash,
            "candidate_conversations_examined":examined,"non_filehelper_conversation_verified":true,"prepared_recipient_verified":true,"prepared_target_is_group":prepared.target_id.ends_with("@chatroom"),"prepared_target_longer_than_inline_string":prepared.target_id.len()>15,"selection_survived_view_close":true,"unverified_target_rejected":true,"closed_view_cannot_prepare":true,
            "first_page_rows":first_count,"first_page_text_rows":text_count,"second_page_rows":second.as_ref().map(|p|p.messages.rows.len()),
            "wrong_cursor_and_filehelper_recovery_rejected":true,"cleared_view_rejected":true,
            "temporary_session_keys_not_persisted":true,"source_db_wal_shm_bytes_unchanged":true,"source_count":paths.len(),
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
    if args.len() != 7 || args[0] != "--consent-prepare-recipient-no-send" {
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
