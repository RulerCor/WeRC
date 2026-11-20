//! One explicitly authorized automatic test in a disposable encrypted product store.
//! Never retry or run automatically: an existing output directory prevents submission.
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
    AppError::new(code, "媒体精确回执验证未通过")
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
    assets: &Path,
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
        .setup_account_for_source("isolated exact media identity verification", &fingerprint)?
        .id;
    let provider = Arc::new(
        ManagedWindowsWechatProvider::with_live_reader(worker.to_owned(), hash.into())
            .with_live_window_limit(120)?,
    );
    let mut app = Application::new(repo.clone(), provider.clone());
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
        let paths = vec![path.clone(), root.join("db_storage/general/general.db")];

        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;

        app.prepare_live_filehelper_local(&account, text, "exact-receipt-source", true)?;

        let media_bytes=|name:&str|->AppResult<Vec<u8>>{std::fs::read(assets.join(name)).map_err(|_|error("OWNED_ASSET_UNAVAILABLE"))};
        let image=app.upload_image_local(&account,media_bytes("frame-0.png")?)?;
        let voice=app.upload_audio_local(&account,media_bytes("automation.wav")?)?;
        let video=app.upload_video_local(&account,"WeRC-自动化测试.mp4",media_bytes("automation.mp4")?)?;
        let emoticon=app.upload_emoticon_local(&account,"WeRC-自动化测试.gif",media_bytes("automation.gif")?)?;
        use werc_application::live_conversation::ConversationCommand as C;
        let parent=app.prepare_live_filehelper_local(&account,text,"exact-batch-quote-parent",true)?;
        let conversation=app.live_conversation_local(&account,C::Open{parent_view_id:parent.view_id,username:"filehelper".into()},"exact-batch-quote-source")?.ok_or_else(||error("SOURCE_PAGE_MISSING"))?;
        let row=conversation.messages.rows.iter().find(|r|r.message_type=="1" && r.text.as_deref().is_some_and(|t|t.starts_with("自动化测试")) && r.server_id.as_deref().is_some_and(|s|s!="0")).ok_or_else(||error("AUTOMATION_QUOTE_SOURCE_MISSING"))?;
        let quote=app.create_quote_send_local(&account,&werc_application::conversation_media::ConversationMediaSelection{view_id:conversation.messages.view_id.clone(),local_id:row.local_id.clone(),source:None},"自动化测试：WeRC 引用消息精确回执与重连验证 2026-11-13，仅此一次，无需回复。","native-exact-batch-quote-20261113-once")?;
        let drafts=vec![
            ("image",app.create_image_send_local(&account,"filehelper",&image.id,"native-exact-batch-image-20261113-once")?),
            ("voice",app.create_voice_send_local(&account,"filehelper",&voice.id,"native-exact-batch-voice-20261113-once")?),
            ("video",app.create_video_send_local(&account,"filehelper",&video.id,"native-exact-batch-video-20261113-once")?),
            ("emoticon",app.create_emoticon_send_local(&account,"filehelper",&emoticon.id,"native-exact-batch-emoticon-20261113-once")?),
            ("quote",quote),
        ];
        let mut submissions=Vec::new();
        let mut evidence=Vec::new();
        for (kind,draft) in &drafts {
            app.approve_text_send_local(&account,&draft.metadata.id,&draft.metadata.request_digest,true)?;
            let marker=output.join(format!("{kind}-submission-started.json"));
            let mut guard=std::fs::OpenOptions::new().write(true).create_new(true).open(marker).map_err(|_|error("ALREADY_ATTEMPTED"))?;
            guard.write_all(serde_json::json!({"outbox_id":draft.metadata.id,"kind":kind,"target":"filehelper","automatic_test":true,"do_not_resend":true}).to_string().as_bytes()).map_err(|_|error("EVIDENCE_WRITE"))?;
            guard.sync_all().map_err(|_|error("EVIDENCE_WRITE"))?;
            // Exactly one execution call per kind, even if the result is unknown.
            let submitted=app.execute_send_local(&account,&draft.metadata.id);
            let mut current=match submitted {
                Ok(value)=>value,
                Err(e)=>{evidence.push(serde_json::json!({"kind":kind,"submitted_error":e.code,"do_not_resend":true}));continue;}
            };
            let mut errors=Vec::new();
            for _ in 0..30 {
                if current.state==werc_domain::outbox::SendState::ServerAccepted {break;}
                std::thread::sleep(std::time::Duration::from_secs(1));
                match app.reconcile_send_local(&account,&draft.metadata.id) {
                    Ok(value)=>current=value,
                    Err(e)=>if !errors.contains(&e.code){errors.push(e.code)},
                }
            }
            let proof=repo.library_get(&account,&format!("send-exact-{}",draft.metadata.id)).is_ok_and(|p|p.header.kind=="send_exact");
            let resubmit_rejected=app.execute_send_local(&account,&draft.metadata.id).is_err();
            submissions.push((kind.to_string(),draft.metadata.id.clone(),current.state));
            evidence.push(serde_json::json!({"kind":kind,"before_reconnect":current.state,"proof_persisted":proof,"resubmit_rejected":resubmit_rejected,"query_errors":errors}));
            std::fs::write(output.join("submissions.json"),serde_json::to_vec_pretty(&evidence).unwrap()).map_err(|_|error("EVIDENCE_WRITE"))?;
        }
        provider.disconnect(None)?;
        app.clear_live_filehelper_local()?;
        let reopened=Arc::new(SqliteRepository::open(&output.join("state.sqlite3"))?);
        app=Application::new(reopened.clone(),provider.clone());
        provider.connect_pinned(&account,host,host_hash,pid,&clients[0].executable_path)?;
        provider.mark_ready(&account)?;
        let before=digests(&paths)?;
        let mut reopened_results=Vec::new();
        for (kind,id,previous) in submissions {
            let after=app.reconcile_send_local(&account,&id);
            match after {
                Ok(value)=>reopened_results.push(serde_json::json!({"kind":kind,"after_reconnect":value.state,"preserved":value.state==previous,"resubmit_rejected":app.execute_send_local(&account,&id).is_err()})),
                Err(e)=>reopened_results.push(serde_json::json!({"kind":kind,"error":e.code})),
            }
        }
        let unchanged=before==digests(&paths)?;
        let passed=evidence.len()==5 && evidence.iter().all(|e|e["before_reconnect"]=="server_accepted" && e["proof_persisted"]==true && e["resubmit_rejected"]==true) && reopened_results.len()==5 && reopened_results.iter().all(|e|e["preserved"]==true && e["resubmit_rejected"]==true);
        Ok(serde_json::json!({"passed":passed,"submissions":evidence,"reopened":reopened_results,"source_db_wal_shm_unchanged_during_reconnect_read":unchanged,"maximum_messages":5,"target":"filehelper","recipient_confirmed":false,"normal_product_state_opened":false,"worker_sha256":hash,"host_sha256":host_hash,"complete_product":false}))
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 8 || args[0] != "--consent-five-filehelper-automation-tests-once" {
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
        Path::new(&args[7]),
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"send_attempted":std::fs::read_dir(output).is_ok_and(|entries|entries.filter_map(Result::ok).any(|e|e.file_name().to_string_lossy().ends_with("-submission-started.json"))),"message":e.message,"do_not_resend":true})
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
