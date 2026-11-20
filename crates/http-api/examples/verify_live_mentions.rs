//! Authenticated group member selection, encrypted draft and real native factory probe. No send.
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
        let paths = vec![
            path.clone(),
            root.join("db_storage/session/session.db"),
            root.join("db_storage/contact/contact.db"),
        ];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, text, "live-sessions-owned-view", true)?;
        use werc_application::live_conversation::ConversationCommand as C;
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
        let members_page = app.live_group_members_local(
            &account,
            &view.view_id,
            &first.username,
            None,
            "live-mention-members",
        )?;
        let members = members_page
            .rows
            .iter()
            .filter_map(|r| r.anchor.username.as_ref())
            .filter(|id| {
                werc_domain::mentions::valid_members(&first.username, std::slice::from_ref(id))
            })
            .take(2)
            .cloned()
            .collect::<Vec<_>>();
        if members.is_empty() {
            return Err(error("NO_AUTHENTICATED_MEMBERS"));
        }
        let text = "自动化测试：WeRC 原生群成员提醒构造验证，不发送";
        let draft = app.create_mention_send_local(
            &account,
            werc_application::mention_send::MentionSelection {
                recipient: RecipientSelection {
                    view_id: first.messages.view_id.clone(),
                    multi: false,
                },
                parent_view_id: view.view_id.clone(),
                cursor: None,
                members: members.clone(),
            },
            text,
            "automation-native-mentions-probe",
        )?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if preview.payload.mentions != members
            || preview.payload.text != text
            || preview.payload.context != selected
            || draft.metadata.schema_version != 7
        {
            return Err(error("MENTION_DRAFT_MISMATCH"));
        }
        let constructed =
            provider.probe_mentions_to(&first.username, text, &preview.payload.mentions)?;
        app.cancel_send_local(&account, &draft.metadata.id)?;
        if !constructed {
            return Err(error("NATIVE_MENTION_CONSTRUCTION_FAILED"));
        }
        if repo.key_revision(&account)? != revision
            || app.sends_local(&account)?.iter().any(|s| {
                s.attempt_id.is_some() || s.state != werc_domain::outbox::SendState::Canceled
            })
        {
            return Err(error("UNEXPECTED_STATE_CHANGE"));
        }
        app.live_conversation_local(
            &account,
            C::Clear {
                view_id: first.messages.view_id.clone(),
            },
            "live-mention-clear",
        )?;
        let unchanged = before == digests(&paths)?;
        if !unchanged {
            return Err(error("SOURCE_CHANGED_DURING_TEST"));
        }
        Ok(
            serde_json::json!({"passed":true,"abi":15,"worker_sha256":hash,"host_sha256":host_hash,
          "candidate_conversations_examined":examined,"authenticated_group":true,"member_count":members.len(),
          "encrypted_draft_and_preview_verified":true,"draft_schema":7,"draft_canceled_without_approval":true,
          "native_factory_member_field_target_and_body_verified":true,"native_send_path_verified":false,"recipient_delivery_verified":false,
          "source_db_wal_shm_bytes_unchanged":true,"source_count":paths.len(),"messages_sent":false,"normal_product_state_opened":false,
          "conversation_records_logged":false,"product_store":"isolated_in_memory","complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-mention-construction-no-send" {
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
