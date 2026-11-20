//! Explicit read-only capture/report/export validation in a disposable encrypted in-memory store.
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
    AppError::new(code, "只读补读与归档验证未通过")
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
        .setup_account_for_source("isolated favorites verification", &fingerprint)?
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
        let paths = vec![path.clone(), root.join("db_storage/favorite/favorite.db")];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, text, "live-sessions-owned-view", true)?;

        let mut next = None;
        let mut pages = 0;
        let mut rows = 0;
        let mut parsed = 0;
        let mut copied = false;
        let mut media = std::collections::BTreeMap::new();
        let mut ids = std::collections::BTreeSet::new();
        loop {
            let page = app.live_favorites_local(
                &account,
                &view.view_id,
                if next.is_none() {
                    Some(Default::default())
                } else {
                    None
                },
                next.as_deref(),
                "live-favorites-read-0001",
            )?;
            pages += 1;
            for row in page.rows {
                rows += 1;
                if !ids.insert(row.anchor.local_id.clone()) {
                    return Err(error("DUPLICATE_FAVORITE"));
                }
                if let Some(content) = row.content {
                    parsed += 1;
                    if !copied {
                        let saved = app.copy_favorite_local(
                            &account,
                            &view.view_id,
                            &row.item_token,
                            "live-favorite-copy-0001",
                        )?;
                        let reopened = Application::new(repo.clone(), provider.clone());
                        let check =
                            reopened.library_item_local(&account, &saved.header.id, "favorite")?;
                        if check.entry.title != saved.entry.title
                            || check.entry.text != saved.entry.text
                        {
                            return Err(error("FAVORITE_COPY_MISMATCH"));
                        }
                        copied = true;
                    }
                    for block in content.blocks {
                        if !matches!(block.kind, 2 | 3 | 4 | 8)
                            || !block.media_available
                            || block.size.is_none_or(|n| n > 1024 * 1024)
                            || media.contains_key(&block.kind)
                        {
                            continue;
                        }
                        let result = app.favorite_media_local(
                            &account,
                            &view.view_id,
                            &row.item_token,
                            &block.slot,
                            "live-favorite-media-0001",
                        );
                        let report = match result {
                            Ok(v) => {
                                serde_json::json!({"ok":true,"bytes":v.file.bytes.len(),"mime":v.mime,"sha256":hex::encode(Sha256::digest(&v.file.bytes))})
                            }
                            Err(e) => {
                                serde_json::json!({"ok":false,"error":e.code,"diagnostic":e.message})
                            }
                        };
                        media.insert(block.kind, report);
                    }
                }
            }
            next = page.next;
            if next.is_none() {
                break;
            }
            if pages >= 20 {
                return Err(error("FAVORITE_PAGE_LIMIT"));
            }
        }
        let after = digests(&paths)?;
        let unchanged = before == after;
        Ok(
            serde_json::json!({"passed":unchanged&&rows==parsed&&copied&&media.values().all(|v|v["ok"]==true),"reading_passed":unchanged&&rows==parsed&&copied,"rows":rows,"parsed":parsed,"pages":pages,"copy_reopened":copied,"media":media,"source_db_wal_shm_bytes_unchanged":unchanged,"worker_sha256":hash,"host_sha256":host_hash,"normal_product_state_opened":false,"messages_sent":false,"record_contents_logged":false,"credentials_persisted":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-favorites" {
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
