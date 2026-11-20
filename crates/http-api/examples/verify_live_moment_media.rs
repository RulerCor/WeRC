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
    AppError::new(code, "只读朋友圈媒体验证未通过")
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
    output: &Path,
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
    let repo = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let account = repo
        .setup_account_for_source("isolated moment media verification", &fingerprint)?
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
        let paths = vec![path.clone(), root.join("db_storage/sns/sns.db")];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;

        let view =
            app.prepare_live_filehelper_local(&account, text, "moment-media-source", true)?;
        let timeline =
            app.query_live_moments_local(&account, &view.view_id, None, None, "moment-media-page")?;
        app.configure_moment_preservation_local(&account, true)?;
        app.step_moment_preservation_local(&account)?;
        app.configure_moment_preservation_local(&account, false)?;
        use werc_domain::moments::*;
        let mut selected = Vec::new();
        for row in &timeline.rows {
            if let Some(content) = &row.content {
                for media in &content.details.media {
                    selected.push((
                        MomentMediaSelection {
                            anchor: row.anchor.clone(),
                            slot: media.slot,
                            variant: MomentMediaVariant::Original,
                        },
                        media.kind == Some(6),
                    ));
                    if media.live_photo {
                        selected.push((
                            MomentMediaSelection {
                                anchor: row.anchor.clone(),
                                slot: media.slot,
                                variant: MomentMediaVariant::LiveVideo,
                            },
                            true,
                        ));
                    }
                }
            }
        }
        // Cover both media families without downloading the entire user's cache.
        selected.sort_by_key(|(s, video)| {
            if s.variant == MomentMediaVariant::LiveVideo {
                0
            } else if *video {
                1
            } else {
                2
            }
        });
        let mut reports = Vec::new();
        let mut saved = Vec::new();
        let (mut images, mut videos, mut live) = (0, 0, 0);
        for (selection, video) in selected.into_iter().take(20) {
            if video && videos > 0 && selection.variant != MomentMediaVariant::LiveVideo {
                continue;
            }
            if !video && images > 1 {
                continue;
            }
            let view =
                app.prepare_live_filehelper_local(&account, text, "moment-media-refresh", true)?;
            match app.fetch_moment_media_local(&account,&view.view_id,&selection,"moment-media-download") {
                Ok(m)=>{
                    let (_,bytes)=app.moment_media_contents_local(&account,&m.id)?;
                    if m.mime=="image/png" {werc_domain::image::canonical_received_png(&bytes)?;images+=1;} else {werc_domain::video::inspect_mp4(&bytes)?;videos+=1;}
                    if selection.variant==MomentMediaVariant::LiveVideo {live+=1;}
                    reports.push(serde_json::json!({"ok":true,"variant":selection.variant,"mime":m.mime,"bytes":m.size,"sha256":m.sha256,"source_md5_matches":m.source_md5_matches}));
                    saved.push((selection,m.sha256));
                }
                Err(e)=>reports.push(serde_json::json!({"ok":false,"variant":selection.variant,"video":video,"error":e.code})),
            }
            if images >= 2 && videos > 0 {
                break;
            }
        }
        let reopened = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
        let offline = Application::new(reopened, provider.clone());
        for (selection, hash) in &saved {
            let m = offline
                .cached_moment_media_local(&account, selection)?
                .ok_or_else(|| error("MEDIA_REOPEN_MISSING"))?;
            let (_, bytes) = offline.moment_media_contents_local(&account, &m.id)?;
            if &hex::encode(Sha256::digest(&*bytes)) != hash {
                return Err(error("MEDIA_REOPEN_MISMATCH"));
            }
        }
        let mut bundle = std::fs::File::create(output.join("offline-moments.zip"))
            .map_err(|_| error("OUTPUT_FAILED"))?;
        let summary = offline.export_moment_bundle_local(
            &account,
            None,
            &Default::default(),
            "moment-media-offline-bundle",
            &mut bundle,
        )?;
        bundle.sync_all().map_err(|_| error("OUTPUT_FAILED"))?;
        let unchanged = before == digests(&paths)?;
        Ok(
            serde_json::json!({"passed":unchanged&&images>0&&summary.media>0,"images":images,"videos":videos,"live_fragments":live,"media":reports,"encrypted_store_reopened":true,"bundle":summary,"source_db_wal_shm_bytes_unchanged":unchanged,"messages_sent":false,"normal_product_state_opened":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-moment-media" {
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
        output,
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
