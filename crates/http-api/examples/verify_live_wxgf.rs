//! Explicit read-only validation of existing cached type8 animations; isolated encrypted store.
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
    AppError::new(code, "只读测试表情验证未通过")
}
fn digest_sources(paths: &[PathBuf]) -> AppResult<Vec<String>> {
    let mut output = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| error("UNSAFE_PATH"))?;
        for file in [
            path.clone(),
            path.with_file_name(format!("{name}-wal")),
            path.with_file_name(format!("{name}-shm")),
        ] {
            output.push(live_digest::digest(&file).map_err(error)?);
        }
    }
    Ok(output)
}
fn run(
    pid: u32,
    worker: &Path,
    worker_hash: &str,
    host: &Path,
    host_hash: &str,
    output: &Path,
) -> AppResult<serde_json::Value> {
    let installations = discovery::discover()?.installations;
    let candidates: Vec<_> = installations
        .iter()
        .filter(|i| i.process_ids.contains(&pid))
        .collect();
    if candidates.len() != 1 || candidates[0].version != "4.1.15.13" {
        return Err(error("CLIENT_SELECTION_REQUIRED"));
    }
    let installation = candidates[0];
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
    let fp = hex::encode(Sha256::digest(
        format!("WeRC/setup-source/v1\0{canonical}").as_bytes(),
    ));
    let repository = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let account = repository
        .setup_account_for_source("isolated wxgf verification", &fp)?
        .id;
    let provider = Arc::new(
        ManagedWindowsWechatProvider::with_live_reader(worker.to_path_buf(), worker_hash.into())
            .with_live_window_limit(120)?,
    );
    let app = Application::new(repository.clone(), provider.clone());
    app.bind_client_local(&account, &installation.executable_path)?;
    provider.connect_pinned(
        &account,
        host,
        host_hash,
        pid,
        &installation.executable_path,
    )?;
    provider.mark_ready(&account)?;
    let result = (|| {
        app.receive_once_local(&account)?;
        let gap = app.receive_cursor_local(&account)?;
        let mut databases: Vec<_> = source
            .databases
            .iter()
            .filter(|d| d.wal_present && named(Path::new(&d.relative_path), "message_"))
            .collect();
        if databases.len() > 16 {
            return Err(error("SOURCE_LIMIT"));
        }
        databases.sort_by(|a, b| b.relative_path.cmp(&a.relative_path));
        for database in databases {
            let path = root.join(&database.relative_path);
            let text = path.to_str().ok_or_else(|| error("UNSAFE_PATH"))?;
            app.bind_live_source_local(&account, text, true)?;
            app.acquire_database_key_local(&account, pid, text, true)?;
            let mut page = match app.prepare_live_filehelper_local(
                &account,
                text,
                "real-wxgf-window-0001",
                true,
            ) {
                Ok(p) => p,
                Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => continue,
                Err(e) => return Err(e),
            };
            let paths = vec![path.clone()];
            let before = digest_sources(&paths)?;
            let key_revision = repository.key_revision(&account)?;
            let mut saved = Vec::new();
            let mut selectors = Vec::<zeroize::Zeroizing<String>>::new();
            for window_index in 0..6 {
                // At most six 120-row windows, each with its own authenticated continuation.
                if let Some(cursor) = &page.older {
                    let next = app.live_filehelper_page_local(
                        &account,
                        Some((cursor, werc_application::live_view::LiveDirection::Older)),
                        60,
                    )?;
                    if next.view_id != page.view_id {
                        return Err(error("VIEW_CHANGED"));
                    }
                    page.rows.extend(next.rows);
                    page.continuation = next.continuation;
                }
                for row in page.rows.iter().filter(|r| {
                    let message = werc_domain::receive::ReceivedText {
                        sequence: 1,
                        native_message_id: r.local_id.clone(),
                        text: String::new(),
                        message_type: (r.message_type.parse::<i64>().unwrap_or(0) & 0xffffffff)
                            as u32,
                        metadata: r.metadata.clone(),
                    };
                    message.message_type == 49
                        && werc_domain::received_emoticon::descriptor(&message).is_ok()
                }) {
                    let descriptor = werc_domain::received_emoticon::descriptor(
                        &werc_domain::receive::ReceivedText {
                            sequence: 1,
                            native_message_id: row.local_id.clone(),
                            text: String::new(),
                            message_type: 49,
                            metadata: row.metadata.clone(),
                        },
                    )?;
                    if selectors.iter().any(|s| **s == *descriptor.md5) {
                        continue;
                    }
                    selectors.push(descriptor.md5);
                    let asset = match app.save_live_emoticon_local(
                        &account,
                        &page.view_id,
                        &row.local_id,
                        "real-wxgf-save-0001",
                    ) {
                        Ok(asset) => asset,
                        Err(e) if e.code == "EMOTICON_CACHE_UNAVAILABLE" => continue,
                        Err(e) => return Err(e),
                    };
                    let restored = app.emoticon_contents_local(&account, &asset.id)?;
                    let stats = preview_stats(&restored.bytes)?;
                    check_known_sample(asset.size, &stats)?;
                    if app.file_preview_local(&account, &asset.id)?.file_name
                        != "received-emoticon.wxgf"
                        || restored.file_name != "received-emoticon.png"
                    {
                        return Err(error("CACHE_PREVIEW_NAME_MISMATCH"));
                    }
                    let preview_hash = hex::encode(Sha256::digest(restored.bytes.as_slice()));
                    if preview_hash == asset.sha256 {
                        return Err(error("CACHE_PREVIEW_NOT_DISTINCT"));
                    }
                    println!(
                        "verified existing animation: {} bytes, {} frames",
                        asset.size, stats["frames"]
                    );
                    saved.push((asset, preview_hash, stats));
                    if saved.len() == 2 {
                        break;
                    }
                }
                println!(
                    "bounded window {} completed; distinct verified animations {}",
                    window_index + 1,
                    saved.len()
                );
                if saved.len() == 2 {
                    break;
                }
                let Some(cursor) = &page.continuation else {
                    break;
                };
                page = app.continue_live_filehelper_local(
                    &account,
                    cursor,
                    "real-wxgf-continue-0001",
                )?;
            }
            if saved.is_empty() {
                continue;
            }
            if saved.len() != 2 {
                return Err(error("TWO_SAMPLES_REQUIRED"));
            }
            if repository.key_revision(&account)? != key_revision {
                return Err(error("KEY_REVISION_CHANGED"));
            }
            if app.receive_cursor_local(&account)? != gap || !app.sends_local(&account)?.is_empty()
            {
                return Err(error("UNEXPECTED_STATE_CHANGE"));
            }
            if before != digest_sources(&paths)? {
                return Err(error("SOURCE_BYTES_CHANGED"));
            }
            return Ok((saved, gap, true, paths.len()));
        }
        Err(error("TEST_EMOTICON_NOT_FOUND"))
    })();
    provider.disconnect(None)?;
    let (saved, gap, unchanged, source_count) = result?;
    app.clear_live_filehelper_local()?;
    drop(app);
    drop(repository);
    drop(provider);
    let repo = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let offline = Application::new(
        repo.clone(),
        Arc::new(werc_wechat_win::WindowsWechatProvider),
    );
    let mut reports = Vec::new();
    for (asset, preview_hash, stats) in saved {
        let preview = offline.emoticon_contents_local(&account, &asset.id)?;
        if hex::encode(Sha256::digest(preview.bytes.as_slice())) != preview_hash
            || preview_stats(&preview.bytes)? != stats
        {
            return Err(error("REOPEN_MISMATCH"));
        }
        reports.push(serde_json::json!({"encrypted_original_bytes":asset.size,"encrypted_original_sha256":asset.sha256,"preview_sha256":preview_hash,"preview_bytes":preview.bytes.len(),"full_frame_stats":stats,"full_preview_equal_after_reopen":true}));
    }
    if offline.receive_cursor_local(&account)? != gap || !offline.sends_local(&account)?.is_empty()
    {
        return Err(error("UNEXPECTED_STATE_CHANGE"));
    }
    drop(offline);
    drop(repo);
    Ok(
        serde_json::json!({"passed":true,"worker_sha256":worker_hash,"host_sha256":host_hash,"emoticons":reports,"source_count":source_count,"source_db_wal_shm_bytes_unchanged":unchanged,"encrypted_store_closed_and_reopened":true,"temporary_media_keys_not_persisted":true,"existing_type49_appmsg8_only":true,"incoming_hook_emoticon_verified":false,"gap_cursor_unchanged":true,"outbox_empty":true,"messages_sent":false,"normal_product_state_opened":false,"receive_hook_emoticon_verified":false,"full_product_completed":false}),
    )
}
fn preview_stats(bytes: &[u8]) -> AppResult<serde_json::Value> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .map_err(|_| error("PREVIEW_INVALID"))?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    let count = info
        .animation_control
        .ok_or_else(|| error("PREVIEW_NOT_ANIMATED"))?
        .num_frames;
    if count > 120 || width > 1024 || height > 1024 {
        return Err(error("PREVIEW_LIMIT"));
    }
    let mut pixels = zeroize::Zeroizing::new(vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| error("PREVIEW_LIMIT"))?
    ]);
    let (mut opaque, mut transparent, mut duration) = (0u64, 0u64, 0u64);
    let mut alpha_hash = 0xcbf29ce484222325u64;
    let mut delays = Vec::new();
    for _ in 0..count {
        let frame = reader
            .next_frame(&mut pixels)
            .map_err(|_| error("PREVIEW_FRAME_INVALID"))?;
        let control = reader
            .info()
            .frame_control
            .ok_or_else(|| error("PREVIEW_TIMING_INVALID"))?;
        if frame.width != width
            || frame.height != height
            || frame.color_type != png::ColorType::Rgba
            || frame.bit_depth != png::BitDepth::Eight
            || control.delay_den != 1000
            || control.delay_num == 0
            || control.blend_op != png::BlendOp::Source
            || control.dispose_op != png::DisposeOp::None
        {
            return Err(error("PREVIEW_PROFILE_MISMATCH"));
        }
        duration += u64::from(control.delay_num);
        delays.push(control.delay_num);
        for px in pixels[..frame.buffer_size()].as_chunks::<4>().0 {
            alpha_hash = (alpha_hash ^ u64::from(px[3])).wrapping_mul(0x100000001b3);
            match px[3] {
                255 => opaque += 1,
                0 if px[..3] == [0, 0, 0] => transparent += 1,
                _ => return Err(error("PREVIEW_ALPHA_INVALID")),
            }
        }
    }
    if reader.next_frame(&mut pixels).is_ok() {
        return Err(error("PREVIEW_EXTRA_FRAME"));
    }
    Ok(
        serde_json::json!({"width":width,"height":height,"frames":count,"duration_ms":duration,"frame_delays_ms":delays,"opaque_pixels":opaque,"transparent_pixels":transparent,"alpha_fnv64":alpha_hash.to_string()}),
    )
}
fn check_known_sample(size: usize, stats: &serde_json::Value) -> AppResult<()> {
    let expected = match size {
        382728 => (
            720,
            91,
            3030,
            9446933u64,
            37727467u64,
            "6677793743322467538",
        ),
        462694 => (280, 68, 2040, 4927370u64, 403830u64, "5299159390498337133"),
        _ => return Err(error("UNRECOGNIZED_EXISTING_SAMPLE")),
    };
    if stats["width"] != expected.0
        || stats["height"] != expected.0
        || stats["frames"] != expected.1
        || stats["duration_ms"] != expected.2
        || stats["opaque_pixels"] != expected.3
        || stats["transparent_pixels"] != expected.4
        || stats["alpha_fnv64"] != expected.5
    {
        return Err(error("INDEPENDENT_RESEARCH_MISMATCH"));
    }
    Ok(())
}
fn named(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix(prefix))
        .and_then(|s| s.strip_suffix(".db"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-existing-cached-animations" {
        std::process::exit(2);
    }
    let output = PathBuf::from(&args[6]);
    assert!(output.is_absolute() && !output.exists());
    std::fs::create_dir(&output).unwrap();
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        Path::new(&args[4]),
        &args[5],
        &output,
    ) {
        Ok(v) => v,
        Err(e) => serde_json::json!({"passed":false,"error":e.code,"messages_sent":false}),
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("evidence.json"))
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    println!("{value}");
    if value["passed"] != true {
        std::process::exit(1);
    }
}
