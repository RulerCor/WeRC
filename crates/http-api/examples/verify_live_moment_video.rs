//! Explicit read-only capture/report/export validation in a disposable encrypted in-memory store.
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
        let mut timeline =
            app.query_live_moments_local(&account, &view.view_id, None, None, "moment-video-page")?;
        let mut next = timeline.next.clone();
        for _ in 0..9 {
            let Some(cursor) = next else { break };
            let page = app.query_live_moments_local(
                &account,
                &view.view_id,
                None,
                Some(&cursor),
                "moment-video-page",
            )?;
            timeline.rows.extend(page.rows);
            next = page.next;
        }
        let (lookup_code, lookup_state) = provider.probe_moment_action(
            &timeline
                .rows
                .first()
                .ok_or_else(|| error("MOMENT_SAMPLE_MISSING"))?
                .anchor
                .id,
        )?;
        let self_id = provider
            .resolve_send_context(&account, "filehelper")?
            .sender_id;
        let owned_test_sns_id = timeline
            .rows
            .iter()
            .find(|r| {
                r.anchor.username == self_id
                    && r.content
                        .as_ref()
                        .is_some_and(|c| c.text.contains("自动化测试"))
            })
            .map(|r| r.anchor.id.clone());
        // Deliberately leave background preservation disabled: manual media
        // capture must retain its own authenticated row for offline export.
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
        selected.retain(|(_, video)| *video);
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
        for (selection, video) in selected {
            if video && videos > 0 && selection.variant != MomentMediaVariant::LiveVideo {
                continue;
            }
            if selection.variant == MomentMediaVariant::LiveVideo && live > 0 {
                continue;
            }
            if !video && images > 1 {
                continue;
            }
            if reports.len() >= 20 {
                break;
            }
            let view =
                app.prepare_live_filehelper_local(&account, text, "moment-media-refresh", true)?;
            match app.fetch_moment_media_local(&account,&view.view_id,&selection,"moment-media-download") {
                Ok(m)=>{
                    let (_,bytes)=app.moment_media_contents_local(&account,&m.id)?;
                    if m.mime=="image/png" {werc_domain::image::canonical_received_png(&bytes)?;images+=1;} else {werc_domain::video::inspect_mp4(&bytes)?; if selection.variant != MomentMediaVariant::LiveVideo {videos+=1;}}
                    if selection.variant==MomentMediaVariant::LiveVideo {live+=1;}
                    reports.push(serde_json::json!({"ok":true,"variant":selection.variant,"mime":m.mime,"bytes":m.size,"sha256":m.sha256,"source_md5_matches":m.source_md5_matches}));
                    saved.push((selection,m.sha256));
                }
                Err(e)=>reports.push(serde_json::json!({"ok":false,"variant":selection.variant,"video":video,"error":e.code})),
            }
            if videos > 0 && live > 0 {
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
        let mut decoded = 0;
        for (selection, _) in &saved {
            let m = offline
                .cached_moment_media_local(&account, selection)?
                .ok_or_else(|| error("MEDIA_REOPEN_MISSING"))?;
            let (_, bytes) = offline.moment_media_contents_local(&account, &m.id)?;
            if m.mime == "video/mp4" {
                let mut process = std::process::Command::new("ffmpeg")
                    .args([
                        "-v", "error", "-xerror", "-i", "pipe:0", "-map", "0:v", "-map", "0:a?",
                        "-f", "null", "-",
                    ])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|_| error("DECODER_UNAVAILABLE"))?;
                process
                    .stdin
                    .take()
                    .ok_or_else(|| error("DECODER_UNAVAILABLE"))?
                    .write_all(&bytes)
                    .map_err(|_| error("DECODE_FAILED"))?;
                if !process
                    .wait()
                    .map_err(|_| error("DECODE_FAILED"))?
                    .success()
                {
                    return Err(error("DECODE_FAILED"));
                }
                decoded += 1;
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
        drop(bundle);
        let mut zip=zip::ZipArchive::new(std::fs::File::open(output.join("offline-moments.zip")).map_err(|_|error("OUTPUT_FAILED"))?).map_err(|_|error("BUNDLE_INVALID"))?;
        let mut bundled_hashes=std::collections::BTreeSet::new();
        for index in 0..zip.len() {
            let mut entry=zip.by_index(index).map_err(|_|error("BUNDLE_INVALID"))?;
            if !entry.name().starts_with("media/") {continue}
            let mut bytes=zeroize::Zeroizing::new(Vec::new());
            std::io::Read::read_to_end(&mut entry,&mut bytes).map_err(|_|error("BUNDLE_INVALID"))?;
            bundled_hashes.insert(hex::encode(Sha256::digest(&*bytes)));
        }
        let bundle_verified=saved.iter().all(|(_,hash)|bundled_hashes.contains(hash)) && summary.media==saved.len() as u64;
        let unchanged = before == digests(&paths)?;
        Ok(
            serde_json::json!({"passed":unchanged&&videos>0&&live>0&&decoded==videos+live&&lookup_code==22&&lookup_state==5&&bundle_verified,"lookup_code":lookup_code,"lookup_state":lookup_state,"owned_test_present":owned_test_sns_id.is_some(),"fully_decoded_after_reopen":decoded,"images":images,"videos":videos,"live_fragments":live,"media":reports,"encrypted_store_reopened":true,"bundle":summary,"bundle_media_hashes_verified":bundle_verified,"background_preservation_enabled":false,"source_db_wal_shm_bytes_unchanged":unchanged,"messages_sent":false,"normal_product_state_opened":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-moment-video" {
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
