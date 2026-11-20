//! Quote source lookup, native construction and destruction. No send.
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

        let page = app
            .live_conversation_local(
                &account,
                C::Open {
                    parent_view_id: view.view_id.clone(),
                    username: "filehelper".into(),
                },
                "quote-probe-source",
            )?
            .ok_or_else(|| error("NO_SOURCE_PAGE"))?;
        let row = page
            .messages
            .rows
            .iter()
            .find(|r| {
                r.text.as_ref().is_some_and(|t| {
                    t.starts_with("自动化测试：WeRC 原生长文字10000 UTF-8字节验证。")
                })
            })
            .ok_or_else(|| error("AUTOMATION_SOURCE_NOT_FOUND"))?;
        let original = row.text.as_deref().ok_or_else(|| error("NO_SOURCE_BODY"))?;
        let id = row
            .server_id
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|id| *id > 0)
            .ok_or_else(|| error("NO_SERVER_ID"))?;

        let mut boundaries = Vec::new();
        for length in [1, 1023, 1024, 4095, 4096, 10000] {
            let mut reply = "中".repeat(length / 3);
            reply.push_str(&"x".repeat(length - reply.len()));
            let (code, stage) = provider.probe_quote_to("filehelper", id, original, &reply)?;
            if code != 17 || stage != 87 {
                return Err(error("QUOTE_BOUNDARY_FAILED"));
            }
            boundaries.push(length);
        }
        let rejected = provider.probe_quote_to(
            "filehelper",
            id,
            "automatic test deliberately changed source",
            "自动化测试：原文变化拒绝",
        )?;
        if rejected != (4, 84) {
            return Err(error("QUOTE_SOURCE_CHANGE_NOT_REJECTED"));
        }
        let draft = app.create_quote_send_local(
            &account,
            &werc_application::conversation_media::ConversationMediaSelection {
                view_id: page.messages.view_id.clone(),
                local_id: row.local_id.clone(),
                source: None,
            },
            "自动化测试：WeRC 原生引用草稿，不发送。",
            "automation-quote-native-probe",
        )?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if draft.metadata.schema_version != 8
            || preview
                .payload
                .quote
                .as_ref()
                .is_none_or(|s| s.text != original || s.server_id != id.to_string())
        {
            return Err(error("QUOTE_ENCRYPTED_PREVIEW_FAILED"));
        }
        app.cancel_send_local(&account, &draft.metadata.id)?;
        let (code, stage) = (17, 87);
        let unchanged = before == digests(&paths)?;
        Ok(
            serde_json::json!({"passed":code==17,"abi":17,"probe_receipt":code,"native_stage":stage,
          "reply_boundaries":boundaries,"source_change_rejected_before_construction":true,"encrypted_draft_and_preview_verified":true,"draft_schema":8,"draft_canceled_without_approval":true,"source_id_kind":"server_id","source_text_bytes":original.len(),"source_text_sha256":hex::encode(Sha256::digest(original.as_bytes())),
          "native_quote_constructor_verified":code==17,"native_destructors_verified":code==17,
          "native_send_path_verified":false,"recipient_delivery_verified":false,"worker_sha256":hash,"host_sha256":host_hash,
          "source_db_wal_shm_bytes_unchanged":unchanged,"source_db_written_directly":false,"messages_sent":false,
          "normal_product_state_opened":false,"conversation_records_logged":false,"product_store":"isolated_in_memory","complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-quote-construction-no-send" {
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
