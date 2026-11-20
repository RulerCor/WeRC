//! One explicitly authorized native quoted filehelper reply. Fixed isolated outbox prevents repetition.
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
    state: &Path,
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
    let repo = Arc::new(SqliteRepository::open(&state.join("state.sqlite3"))?);
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
        let _before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, text, "live-sessions-owned-view", true)?;
        use werc_application::live_conversation::ConversationCommand as C;
        let revision = repo.key_revision(&account)?;
        let selected = provider.resolve_send_context(&account, "filehelper")?;

        let page = app
            .live_conversation_local(
                &account,
                C::Open {
                    parent_view_id: view.view_id.clone(),
                    username: "filehelper".into(),
                },
                "quote-once-select-source",
            )?
            .ok_or_else(|| error("SOURCE_PAGE_MISSING"))?;
        let row = page
            .messages
            .rows
            .iter()
            .find(|r| {
                r.text.as_ref().is_some_and(|t| {
                    t.starts_with("自动化测试：WeRC 原生长文字10000 UTF-8字节验证。")
                })
            })
            .ok_or_else(|| error("AUTOMATION_SOURCE_MISSING"))?;
        let original = row
            .text
            .as_deref()
            .ok_or_else(|| error("SOURCE_BODY_MISSING"))?;
        let id = row
            .server_id
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|id| *id > 0)
            .ok_or_else(|| error("SOURCE_ID_MISSING"))?;
        let text = "自动化测试：WeRC 原生引用回复验证。引用上方已发送的长文字测试消息，仅此一次，无需回复。2026-11-12";
        let draft = app.create_quote_send_local(
            &account,
            &werc_application::conversation_media::ConversationMediaSelection {
                view_id: page.messages.view_id.clone(),
                local_id: row.local_id.clone(),
                source: None,
            },
            text,
            "automation-quote-once-20261112",
        )?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if preview.payload.text != text
            || preview.payload.context != selected
            || draft.metadata.schema_version != 8
            || preview
                .payload
                .quote
                .as_ref()
                .is_none_or(|q| q.text != original || q.server_id != id.to_string())
        {
            return Err(error("QUOTE_DRAFT_MISMATCH"));
        }
        if provider.probe_quote_to("filehelper", id, original, text)? != (17, 87) {
            return Err(error("QUOTE_CONSTRUCTION_FAILED"));
        }
        if repo.key_revision(&account)? != revision {
            return Err(error("KEY_REVISION_CHANGED"));
        }
        app.approve_text_send_local(
            &account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
        )?;
        // Create-once marker is durable before any native submission. Never retry.
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(state.join("submission-started.json"))
            .map_err(|_| error("ATTEMPT_ALREADY_STARTED"))?;
        marker.write_all(b"{\"send_attempt_started\":true,\"automatic_retry\":false,\"filehelper_only\":true}").map_err(|_|AppError::internal())?;
        marker.sync_all().map_err(|_| AppError::internal())?;
        let mut status = app.execute_send_local(&account, &draft.metadata.id)?;
        let mut callback = false;
        for _ in 0..30 {
            if status.state != werc_domain::outbox::SendState::Unknown {
                break;
            }
            status = app.reconcile_send_local(&account, &draft.metadata.id)?;
            let receipt = provider.reconcile_text(
                werc_domain::outbox::TextReceiptQuery {
                    outbox_id: &draft.metadata.id,
                    attempt_id: status
                        .attempt_id
                        .as_deref()
                        .ok_or_else(AppError::internal)?,
                    context_digest: &status.metadata.context_digest,
                    context: &preview.payload.context,
                },
                None,
            )?;
            if receipt.provider_reference.as_deref() == Some("native-callback:2") {
                callback = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let mut observed = false;
        let mut server_id_present = false;
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if let Some(page) = app.live_conversation_local(
                &account,
                C::Open {
                    parent_view_id: view.view_id.clone(),
                    username: "filehelper".into(),
                },
                "live-long-text-readback",
            )? && let Some(row) = page.messages.rows.iter().find(|r| {
                r.sender_username.as_deref()==Some(selected.sender_id.as_str()) &&
                page.cards.iter().any(|card| card.local_id==r.local_id && matches!(&card.content,
                    werc_domain::message_card::DisplayContent::Card(werc_domain::message_card::MessageCard::Quote{text:reply,quoted_type:1,quoted_text:Some(quoted),..}) if reply==text && quoted==original))
            }) {
                observed = true;
                server_id_present = row.server_id.as_ref().is_some_and(|s| s != "0");
                break;
            }
        }
        Ok(
            serde_json::json!({"passed":callback,"abi":17,"worker_sha256":hash,"host_sha256":host_hash,
          "filehelper_only":true,"quoted_source_bytes":original.len(),"quoted_source_sha256":hex::encode(Sha256::digest(original.as_bytes())),"text_bytes":text.len(),"text_sha256":hex::encode(Sha256::digest(text.as_bytes())),
          "encrypted_draft_and_preview_verified":true,"draft_schema":8,"send_attempts":1,"native_callback_verified":callback,
          "outbox_state":status.state,"local_quote_card_with_exact_reply_and_source_observed":observed,"server_id_present_on_local_message":server_id_present,
          "recipient_delivery_verified":false,"source_db_written_directly":false,"source_bytes_may_change_due_to_native_send":true,
          "messages_sent":true,"normal_product_state_opened":false,"conversation_records_logged":false,"product_store":"isolated_encrypted_sqlite","complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 8 || args[0] != "--consent-one-filehelper-quote-automation-test" {
        std::process::exit(2);
    }
    let state = Path::new(&args[7]);
    std::fs::create_dir(state).expect("fixed isolated state must not already exist; never resend");
    let output = Path::new(&args[6]);
    assert!(output.is_absolute() && !output.exists());
    std::fs::create_dir(output).unwrap();
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        Path::new(&args[4]),
        &args[5],
        state,
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"send_attempt_started":state.join("submission-started.json").exists(),"automatic_retry":false})
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
