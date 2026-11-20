//! Explicit read-only type49 display-card validation. Only aggregate evidence leaves memory.
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
    AppError::new(code, "只读会话卡片验证未通过")
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
        app.receive_once_local(&account)?;
        let gap = app.receive_cursor_local(&account)?;
        let mut candidates: Vec<_> = source
            .databases
            .iter()
            .filter(|d| {
                d.wal_present
                    && Path::new(&d.relative_path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .and_then(|n| n.strip_prefix("message_"))
                        .and_then(|n| n.strip_suffix(".db"))
                        .and_then(|n| n.parse::<u32>().ok())
                        .is_some()
            })
            .collect();
        candidates.sort_by_key(|d| {
            std::cmp::Reverse(
                Path::new(&d.relative_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_prefix("message_"))
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0),
            )
        });
        if candidates.is_empty() {
            return Err(error("SOURCE_UNAVAILABLE"));
        }
        let paths: Vec<_> = candidates
            .iter()
            .take(2)
            .map(|d| root.join(&d.relative_path))
            .collect();
        let before = digests(&paths)?;
        for path in &paths {
            let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
            app.bind_live_source_local(&account, text, true)?;
            app.acquire_database_key_local(&account, pid, text, true)?;
        }
        use werc_application::{
            live_conversation::ConversationCommand as C, live_view::LiveDirection,
        };
        use werc_domain::message_card::MessageCard;
        let mut reports = Vec::new();
        for multi in [false, true] {
            let mut single = None;
            let mut merged = None;
            if multi {
                let parent = app.prepare_live_filehelper_multi_local(
                    &account,
                    &paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect::<Vec<_>>(),
                    "real-cards-multi-parent",
                    true,
                )?;
                merged = app.live_multi_conversation_local(
                    &account,
                    C::Open {
                        parent_view_id: parent.view_id,
                        username: "filehelper".into(),
                    },
                    "real-cards-multi-open",
                )?;
            } else {
                let parent = app.prepare_live_filehelper_local(
                    &account,
                    paths[0].to_str().ok_or_else(|| error("SOURCE_PATH"))?,
                    "real-cards-single-parent",
                    true,
                )?;
                single = app.live_conversation_local(
                    &account,
                    C::Open {
                        parent_view_id: parent.view_id,
                        username: "filehelper".into(),
                    },
                    "real-cards-single-open",
                )?;
            }
            let mut counts = [0; 3];
            let mut pages = 0;
            let mut selected = std::collections::HashSet::new();
            let mut records = 0;
            let mut quoted_text = 0;
            for _ in 0..16 {
                let (view, cards, older, continuation) = if let Some(p) = &merged {
                    if p.messages.rows.iter().any(|r| r.message.metadata.is_some())
                        || p.cards.iter().any(|c| {
                            !p.messages.rows.iter().any(|r| {
                                r.source == c.source.as_deref().unwrap_or("")
                                    && r.message.local_id == c.local_id
                            })
                        })
                    {
                        return Err(error("PUBLIC_IDENTITY_MISMATCH"));
                    }
                    (
                        &p.messages.view_id,
                        &p.cards,
                        &p.messages.older,
                        &p.messages.continuation,
                    )
                } else {
                    let p = single.as_ref().ok_or_else(|| error("VIEW_MISSING"))?;
                    if p.messages.rows.iter().any(|r| r.metadata.is_some())
                        || p.cards.iter().any(|c| {
                            c.source.is_some()
                                || !p.messages.rows.iter().any(|r| r.local_id == c.local_id)
                        })
                    {
                        return Err(error("PUBLIC_IDENTITY_MISMATCH"));
                    }
                    (
                        &p.messages.view_id,
                        &p.cards,
                        &p.messages.older,
                        &p.messages.continuation,
                    )
                };
                pages += 1;
                for card in cards {
                    if !selected.insert((card.source.clone(), card.local_id.clone())) {
                        return Err(error("DUPLICATE_CARD"));
                    }
                    let werc_domain::message_card::DisplayContent::Card(content) = &card.content else { continue };
                    match content {
                        MessageCard::Contact { .. }
                        | MessageCard::Location { .. }
                        | MessageCard::AppShare { .. } => {}
                        MessageCard::Link { .. } => counts[0] += 1,
                        MessageCard::Quote {
                            quoted_text: text, ..
                        } => {
                            counts[1] += 1;
                            quoted_text += usize::from(text.is_some());
                        }
                        MessageCard::Forward { entries, .. } => {
                            counts[2] += 1;
                            records += entries.len();
                        }
                    }
                }
                if counts.iter().all(|c| *c > 0) {
                    break;
                }
                let next = if let Some(cursor) = older {
                    C::Page {
                        view_id: view.clone(),
                        cursor: cursor.clone(),
                        direction: LiveDirection::Older,
                    }
                } else if let Some(cursor) = continuation {
                    C::Continue {
                        view_id: view.clone(),
                        cursor: cursor.clone(),
                    }
                } else {
                    break;
                };
                if multi {
                    merged =
                        app.live_multi_conversation_local(&account, next, "real-cards-multi-next")?;
                } else {
                    single =
                        app.live_conversation_local(&account, next, "real-cards-single-next")?;
                }
            }
            if counts.iter().sum::<usize>() == 0 {
                return Err(error("NO_CARDS_IN_BOUNDED_SELECTION"));
            }
            reports.push(serde_json::json!({"multi":multi,"pages":pages,"links":counts[0],"quotes":counts[1],"forwarded_records":counts[2],"forward_entries":records,"text_quotes":quoted_text,"all_three_kinds_present":counts.iter().all(|c|*c>0)}));
        }
        if gap != app.receive_cursor_local(&account)? || !app.sends_local(&account)?.is_empty() {
            return Err(error("UNEXPECTED_STATE_CHANGE"));
        }
        let unchanged = before == digests(&paths)?;
        Ok(
            serde_json::json!({"passed":unchanged,"worker_sha256":hash,"host_sha256":host_hash,"reports":reports,"source_db_wal_shm_bytes_unchanged":unchanged,"source_count":paths.len(),"public_metadata_removed":true,"exact_source_local_id_verified":true,"messages_sent":false,"outbox_empty":true,"gap_cursor_unchanged":true,"normal_product_state_opened":false,"card_contents_logged":false,"product_store":"isolated_in_memory","complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-message-cards" {
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
