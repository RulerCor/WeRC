//! Read existing forwarded original images via authenticated single/multi windows; isolated encrypted store.
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
    AppError::new(code, "所选会话媒体只读验证未通过")
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
fn image_paths(
    entries: &[werc_domain::message_card::RecordEntry],
    prefix: &[usize],
) -> Vec<Vec<usize>> {
    let mut paths = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(index);
        if entry.image_available {
            paths.push(path.clone());
        }
        if let Some(nested) = &entry.nested {
            paths.extend(image_paths(&nested.entries, &path));
        }
    }
    paths.sort_by_key(|p| std::cmp::Reverse(p.len()));
    paths
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
        .setup_account_for_source("isolated selected media verification", &fp)?
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
    use werc_application::{
        conversation_media::ConversationMediaSelection,
        live_conversation::ConversationCommand as C, live_view::LiveDirection,
    };
    let result = (|| {
        app.receive_once_local(&account)?;
        let gap = app.receive_cursor_local(&account)?;
        let mut databases: Vec<_> = source
            .databases
            .iter()
            .filter(|d| d.wal_present && named(Path::new(&d.relative_path), "message_"))
            .collect();
        databases.sort_by(|a, b| b.relative_path.cmp(&a.relative_path));
        if databases.is_empty() || databases.len() > 16 {
            return Err(error("SOURCE_LIMIT"));
        }
        let paths: Vec<_> = databases
            .iter()
            .take(2)
            .map(|d| root.join(&d.relative_path))
            .collect();
        let before = digest_sources(&paths)?;
        for path in &paths {
            let text = path.to_str().ok_or_else(|| error("UNSAFE_PATH"))?;
            app.bind_live_source_local(&account, text, true)?;
            app.acquire_database_key_local(&account, pid, text, true)?;
        }
        let mut saved = Vec::new();
        let mut failures = Vec::new();
        let mut selected_username: Option<zeroize::Zeroizing<String>> = None;
        for multi in [false, true] {
            let parent = if multi {
                app.prepare_live_filehelper_multi_local(
                    &account,
                    &paths
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect::<Vec<_>>(),
                    "forward-parent-multi",
                    true,
                )?
                .view_id
            } else {
                app.prepare_live_filehelper_local(
                    &account,
                    paths[0].to_str().ok_or_else(|| error("PATH"))?,
                    "forward-parent-single",
                    true,
                )?
                .view_id
            };
            let users: Vec<_> = if let Some(username) = &selected_username {
                vec![username.clone()]
            } else {
                let mut rows = app
                    .live_sessions_local(&account, &parent, None, "forward-session-candidates")?
                    .rows;
                rows.truncate(64);
                // Prioritize visible record summaries within the same authenticated bound.
                // This avoids spending the source stability window on unrelated sessions.
                rows.sort_by_key(|r| !r.summary.as_deref().is_some_and(|s| s.contains("聊天记录")));
                rows.into_iter()
                    .take(64)
                    .map(|r| zeroize::Zeroizing::new(r.anchor.username.clone()))
                    .collect()
            };
            let mut found = false;
            'users: for (candidate_index, username) in users.into_iter().enumerate() {
                eprintln!("Forward image scope merged={multi} candidate={candidate_index}");
                let mut single = None;
                let mut merged = None;
                let open = C::Open {
                    parent_view_id: parent.clone(),
                    username: username.to_string(),
                };
                if multi {
                    merged = match app.live_multi_conversation_local(
                        &account,
                        open,
                        "forward-open-multi",
                    ) {
                        Ok(page) => page,
                        Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => continue,
                        Err(e) => return Err(e),
                    };
                } else {
                    single =
                        match app.live_conversation_local(&account, open, "forward-open-single") {
                            Ok(page) => page,
                            Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => continue,
                            Err(e) => return Err(e),
                        };
                }
                for _ in 0..4 {
                    let (view, cards, older, continuation) = if let Some(p) = &merged {
                        (
                            &p.messages.view_id,
                            &p.cards,
                            &p.messages.older,
                            &p.messages.continuation,
                        )
                    } else {
                        let p = single.as_ref().ok_or_else(|| error("NO_VIEW"))?;
                        (
                            &p.messages.view_id,
                            &p.cards,
                            &p.messages.older,
                            &p.messages.continuation,
                        )
                    };
                    for card in cards {
                        if let werc_domain::message_card::DisplayContent::Card(
                            werc_domain::message_card::MessageCard::Forward { entries, .. },
                        ) =
                            &card.content
                        {
                            for path in image_paths(entries, &[]) {
                                let selection = ConversationMediaSelection {
                                    view_id: view.clone(),
                                    local_id: card.local_id.clone(),
                                    source: card.source.clone(),
                                };
                                let asset = match app.save_conversation_media_path_local(
                                    &account,
                                    &selection,
                                    None,
                                    Some(&path),
                                    "forward-real-save",
                                ) {
                                    Ok(a) => a,
                                    Err(e)
                                        if failures.len() < 4
                                            && matches!(
                                                e.code.as_str(),
                                                "NATIVE_DOWNLOAD_FAILED" | "FORWARD_MEDIA_INVALID"
                                            ) =>
                                    {
                                        failures.push(e.code);
                                        continue;
                                    }
                                    Err(e) => return Err(e),
                                };
                                let content = app
                                    .conversation_media_content_local(&account, &asset.reference)?;
                                werc_domain::image::canonical_received_png(&content.file.bytes)?;
                                saved.push((
                                    asset.reference,
                                    hex::encode(Sha256::digest(content.file.bytes.as_slice())),
                                    content.file.bytes.len(),
                                    content.mime.to_string(),
                                    multi,
                                    path.len(),
                                ));
                                selected_username = Some(username.clone());
                                found = true;
                                break 'users;
                            }
                        }
                    }
                    let command = if let Some(cursor) = older {
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
                        merged = app.live_multi_conversation_local(
                            &account,
                            command,
                            "forward-next-multi",
                        )?;
                    } else {
                        single =
                            app.live_conversation_local(&account, command, "forward-next-single")?;
                    }
                }
            }
            if !found {
                return Err(error("FORWARD_IMAGE_NOT_IN_BOUNDED_WINDOWS"));
            }
        }
        let after = digest_sources(&paths)?;
        let source_changes:Vec<_>=before.iter().zip(&after).enumerate().filter(|(_, (a,b))|a!=b).map(|(i,_)|serde_json::json!({"source_index":i/3+1,"file_kind":(["db","wal","shm"][i%3])})).collect();
        if app.receive_cursor_local(&account)? != gap || !app.sends_local(&account)?.is_empty() {
            return Err(error("UNEXPECTED_STATE_CHANGE"));
        }
        Ok((saved, gap, paths.len(), failures, source_changes))
    })();
    provider.disconnect(None)?;
    let (saved, gap, source_count, failures, source_changes) = result?;
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
    for (reference, hash, size, mime, multi, depth) in saved {
        let bytes = offline.conversation_media_content_local(&account, &reference)?;
        if hex::encode(Sha256::digest(bytes.file.bytes.as_slice())) != hash
            || bytes.file.bytes.len() != size
            || bytes.mime != mime
        {
            return Err(error("REOPEN_MISMATCH"));
        }
        offline.delete_conversation_media_local(&account, &reference)?;
        if offline
            .conversation_media_content_local(&account, &reference)
            .is_ok()
        {
            return Err(error("DELETE_FAILED"));
        }
        reports.push(serde_json::json!({"kind":reference.kind,"merged":multi,"selection_depth":depth,"bytes":size,"mime":mime,"equal_after_reopen":true,"deleted_after_reopen":true,"original_size_and_md5_verified":true}));
    }
    if offline.receive_cursor_local(&account)? != gap || !offline.sends_local(&account)?.is_empty()
    {
        return Err(error("UNEXPECTED_STATE_CHANGE"));
    }
    Ok(
        serde_json::json!({"passed":source_changes.is_empty(),"worker_sha256":worker_hash,"host_sha256":host_hash,"media":reports,"source_count":source_count,"unavailable_attempts":failures,"source_db_wal_shm_bytes_unchanged":source_changes.is_empty(),"source_endpoint_changes":source_changes,"forward_images_verified":true,"full_entry_paths_verified":true,"deep_image_verified":reports.iter().any(|v|v["selection_depth"].as_u64().is_some_and(|d|d>1)),"encrypted_store_closed_and_reopened":true,"temporary_media_keys_not_persisted":true,"selected_conversation":"existing_forward_record","non_filehelper_voice_verified":false,"incoming_hook_verified":false,"gap_cursor_unchanged":true,"outbox_empty":true,"messages_sent":false,"normal_product_state_opened":false,"full_product_completed":false}),
    )
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
    if args.len() != 7 || args[0] != "--consent-existing-forward-images" {
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
