//! Read-only recipient metadata from the source-built worker; aggregate evidence only.
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
    AppError::new(code, "只读提及字段验证未通过")
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
    _output: &Path,
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
        .setup_account_for_source("isolated public avatar verification", &fingerprint)?
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
        let paths = vec![path.clone(), root.join("db_storage/contact/contact.db")];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, text, "live-sessions-owned-view", true)?;

        let login = provider
            .resolve_send_context(&account, "filehelper")?
            .sender_id;
        let sessions =
            app.live_sessions_local(&account, &view.view_id, None, "mentions-local-sessions")?;
        let mut total = 0usize;
        let mut available = 0usize;
        let mut recipient_rows = 0usize;
        let mut self_mentions = 0usize;
        let mut all_mentions = 0usize;
        let mut groups = 0usize;
        for group in sessions
            .rows
            .iter()
            .filter(|s| werc_domain::live::is_group_account(&s.anchor.username))
            .take(8)
        {
            let page = app
                .live_conversation_local(
                    &account,
                    werc_application::live_conversation::ConversationCommand::Open {
                        parent_view_id: view.view_id.clone(),
                        username: group.anchor.username.clone(),
                    },
                    "mentions-local-conversation",
                )?
                .ok_or_else(|| error("CONVERSATION_MISSING"))?;
            groups += 1;
            for message in &page.messages.rows {
                total += 1;
                if let Some(users) = &message.at_user_list {
                    available += 1;
                    recipient_rows += usize::from(!users.is_empty());
                    self_mentions += usize::from(users.iter().any(|u| u == &login));
                    all_mentions += usize::from(users.iter().any(|u| u == "notify@all"));
                }
            }
            app.live_conversation_local(
                &account,
                werc_application::live_conversation::ConversationCommand::Clear {
                    view_id: page.messages.view_id,
                },
                "mentions-local-clear",
            )?;
        }
        let unchanged = before == digests(&paths)?;
        Ok(
            serde_json::json!({"passed":groups>0&&total>0&&available>0&&unchanged&&app.sends_local(&account)?.is_empty(),"abi":26,"worker_sha256":hash,"host_sha256":host_hash,"groups":groups,"messages":total,"parsed_source_metadata":available,"rows_with_recipients":recipient_rows,"exact_self_mentions":self_mentions,"all_mentions":all_mentions,"source_db_wal_shm_bytes_unchanged":unchanged,"messages_sent":false,"normal_product_state_opened":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-mention-metadata" {
        std::process::exit(2);
    }
    let output = Path::new(&args[6]);
    assert!(output.is_absolute() && !output.exists());
    std::fs::create_dir(output).unwrap();
    let value = match run(
        output,
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        Path::new(&args[4]),
        &args[5],
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"detail":e.message,"messages_sent":false})
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
