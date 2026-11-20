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
        conversation_media::{ConversationMediaKind as K, ConversationMediaSelection},
        live_conversation::ConversationCommand as C,
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
        let d = databases.first().ok_or_else(|| error("NO_SOURCE"))?;
        let path = root.join(&d.relative_path);
        let text = path.to_str().ok_or_else(|| error("UNSAFE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let parent =
            app.prepare_live_filehelper_local(&account, text, "non-helper-parent", true)?;
        let sessions =
            app.live_sessions_local(&account, &parent.view_id, None, "non-helper-sessions")?;
        let before = digest_sources(std::slice::from_ref(&path))?;
        let mut examined = 0;
        let mut candidates: Vec<_> = sessions
            .rows
            .iter()
            .filter(|s| s.anchor.username != "filehelper")
            .collect();
        candidates.sort_by_key(|s| {
            (
                !s.summary.as_deref().is_some_and(|v| v.contains("语音")),
                s.anchor.username.ends_with("@chatroom"),
            )
        });
        for session in candidates.into_iter().take(32) {
            examined += 1;
            let page = match app.live_conversation_local(
                &account,
                C::Open {
                    parent_view_id: parent.view_id.clone(),
                    username: session.anchor.username.clone(),
                },
                "non-helper-conversation",
            ) {
                Ok(Some(p)) => p,
                Ok(None) => continue,
                Err(e) if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE" => continue,
                Err(e) => return Err(e),
            };
            if page.username == "filehelper" {
                return Err(error("WRONG_CONVERSATION"));
            }
            for hint in page.media.iter().filter(|m| m.kind == K::Voice).take(3) {
                let asset = match app.save_conversation_media_local(
                    &account,
                    &ConversationMediaSelection {
                        view_id: page.messages.view_id.clone(),
                        local_id: hint.local_id.clone(),
                        source: None,
                    },
                    "non-helper-voice-save",
                ) {
                    Ok(a) => a,
                    Err(e) if e.code == "LIVE_VOICE_NOT_CACHED" => continue,
                    Err(e) => return Err(e),
                };
                let bytes = app.conversation_media_content_local(&account, &asset.reference)?;
                let audio = werc_domain::media::inspect_wav(&bytes.file.bytes)?;
                let hash = hex::encode(Sha256::digest(bytes.file.bytes.as_slice()));
                if before != digest_sources(std::slice::from_ref(&path))? {
                    return Err(error("SOURCE_BYTES_CHANGED"));
                }
                if app.receive_cursor_local(&account)? != gap
                    || !app.sends_local(&account)?.is_empty()
                {
                    return Err(error("UNEXPECTED_STATE_CHANGE"));
                }
                return Ok((
                    asset.reference,
                    hash,
                    audio,
                    bytes.file.bytes.len(),
                    gap,
                    examined,
                    page.username.ends_with("@chatroom"),
                ));
            }
        }
        Err(error("NO_CACHED_VOICE_IN_BOUNDED_SELECTION"))
    })();
    provider.disconnect(None)?;
    let (reference, hash, audio, size, gap, examined, group) = result?;
    app.clear_live_filehelper_local()?;
    drop(app);
    drop(repository);
    drop(provider);
    let repo = Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
    let offline = Application::new(
        repo.clone(),
        Arc::new(werc_wechat_win::WindowsWechatProvider),
    );
    let bytes = offline.conversation_media_content_local(&account, &reference)?;
    if hex::encode(Sha256::digest(bytes.file.bytes.as_slice())) != hash
        || bytes.file.bytes.len() != size
        || werc_domain::media::inspect_wav(&bytes.file.bytes)? != audio
    {
        return Err(error("REOPEN_MISMATCH"));
    }
    if offline.receive_cursor_local(&account)? != gap || !offline.sends_local(&account)?.is_empty()
    {
        return Err(error("UNEXPECTED_STATE_CHANGE"));
    }
    offline.delete_conversation_media_local(&account, &reference)?;
    if offline
        .conversation_media_content_local(&account, &reference)
        .is_ok()
    {
        return Err(error("DELETE_FAILED"));
    }
    Ok(
        serde_json::json!({"passed":true,"worker_sha256":worker_hash,"host_sha256":host_hash,"non_filehelper_voice_verified":true,"group_conversation":group,"bounded_sessions_examined":examined,"bytes":size,"duration_ms":audio.duration_ms,"full_decode":true,"byte_equal_after_encrypted_reopen":true,"deleted_after_verification":true,"source_db_wal_shm_bytes_unchanged":true,"gap_cursor_unchanged":true,"outbox_empty":true,"messages_sent":false,"normal_product_state_opened":false,"identifiers_or_content_logged":false,"complete_product":false}),
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
    if args.len() != 7 || args[0] != "--consent-existing-non-filehelper-voice" {
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
