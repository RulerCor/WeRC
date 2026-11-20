//! Explicit read-only capture/report/export validation in an isolated encrypted disk store.
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
    AppError::new(code, "只读真实资料关联验证未通过")
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
    output_directory: &Path,
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
    let repo = Arc::new(SqliteRepository::open(&output_directory.join("state.sqlite3"))?);
    let account = repo
        .setup_account_for_source("isolated capture verification", &fingerprint)?
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
        let paths = vec![path.clone(), root.join("db_storage/session/session.db"), root.join("db_storage/sns/sns.db")];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, text, "live-sessions-owned-view", true)?;
        let newest = view
            .rows
            .iter()
            .map(|r| r.timestamp)
            .max()
            .ok_or_else(|| error("NO_SAMPLE"))?;
        let from = newest.saturating_sub(86400);
        let until = newest + 1;
        let mut capture =
            app.start_source_capture_local(&account, &view.view_id, Some(from), Some(until))?;
        let mut iterations = 0;
        while !capture.complete_selected_sources && iterations < 200 {
            capture =
                app.step_source_capture_local(&account, &capture.id, "live-capture-step-0001")?;
            iterations += 1;
        }
        if !capture.complete_selected_sources || capture.rows == 0 {
            return Err(error("CAPTURE_INCOMPLETE"));
        }
        app.configure_moment_preservation_local(&account,true)?;
        let preserved=app.step_moment_preservation_local(&account)?;
        if preserved.error.is_some() || preserved.saved==0 {return Err(error("SOCIAL_SAMPLE_MISSING"));}
        app.configure_moment_preservation_local(&account,false)?;
        let reopened_repository=Arc::new(SqliteRepository::open(&output_directory.join("state.sqlite3"))?);
        let reopened = Application::new(reopened_repository, provider.clone());
        let recovered = reopened.source_capture_status_local(&account, &capture.id)?;
        if recovered.pages != capture.pages || recovered.rows != capture.rows {
            return Err(error("CHECKPOINT_MISMATCH"));
        }
        let report = reopened.capture_report_local(
            &account,
            &capture.id,
            &werc_application::capture_tools::CaptureFilter::default(),
            "live-capture-report-0001",
        )?;
        if report.counted_rows != capture.rows
            || report.sent + report.received + report.unknown_sender != capture.rows
        {
            return Err(error("REPORT_MISMATCH"));
        }
        use werc_application::capture_timeline::{TimelineMode,TimelineQuery};
        let mut timelines=Vec::new();
        for mode in [TimelineMode::All,TimelineMode::Private,TimelineMode::WaitingMe,TimelineMode::WaitingThem,TimelineMode::Mentions] {
            let query=TimelineQuery{mode,..Default::default()};
            let timeline=reopened.capture_timeline_local(&account,&capture.id,&query,None,"real-social-timeline")?;
            let mut next_rows=0;
            if let Some(cursor)=&timeline.next {
                let next=reopened.capture_timeline_local(&account,&capture.id,&query,Some(cursor),"real-social-timeline-next")?;
                let last=timeline.data.last().ok_or_else(||error("TIMELINE_CURSOR_WITHOUT_ROWS"))?;
                if next.data.iter().any(|r|(r.timestamp,r.page,r.row)>=(last.timestamp,last.page,last.row)){return Err(error("TIMELINE_ORDER"));}
                next_rows=next.data.len();
            }
            timelines.push(serde_json::json!({"mode":mode,"first_page":timeline.data.len(),"next_page":next_rows,"scanned":timeline.scanned,"mentions":timeline.summary.mentions}));
        }
        let filter=werc_application::capture_tools::CaptureFilter::default();
        let social=reopened.capture_social_report_local(&account,&capture.id,&filter,"real-social-report")?;
        if social.unreadable_posts!=0 || social.unmatched_login_posts!=0 || social.observed_posts==0 {return Err(error("SOCIAL_RECORDS_INCOMPLETE"));}
        let mut exports=Vec::new();
        for format in ["json","txt","html"] {
            let bytes=zeroize::Zeroizing::new(reopened.export_capture_social_local(&account,&capture.id,&filter,format,"real-social-export")?);
            if bytes.is_empty(){return Err(error("SOCIAL_EXPORT_EMPTY"));}
            if format=="json" {let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|error("SOCIAL_EXPORT_INVALID"))?;if value["contacts"].as_array().map(Vec::len)!=Some(social.contacts.len()){return Err(error("SOCIAL_EXPORT_COUNT"));}}
            exports.push(serde_json::json!({"format":format,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(&*bytes))}));
        }
        let mut located=false;
        for index in 0..capture.pages {
            let page=reopened.source_capture_page_local(&account,&capture.id,index)?;
            for row in &page.rows {
                if let Some(id)=row.message.server_id.as_deref().filter(|s|*s!="0") {
                    let found=reopened.capture_message_location_local(&account,&capture.id,&row.username,id,"real-social-locate")?;
                    let target=reopened.source_capture_page_local(&account,&capture.id,found.position.page)?;
                    let matched=target.rows.get(found.position.row).ok_or_else(||error("LOCATOR_ROW_MISSING"))?;
                    if matched.username!=row.username || matched.message.server_id.as_deref()!=Some(id) {return Err(error("LOCATOR_IDENTITY_MISMATCH"));}
                    located=true;break;
                }
            }
            if located {break;}
        }
        if !located{return Err(error("LOCATOR_SAMPLE_MISSING"));}
        if !app.sends_local(&account)?.is_empty() {
            return Err(error("UNEXPECTED_SEND"));
        }
        let unchanged = before == digests(&paths)?;
        if !unchanged {
            return Err(error("SOURCE_CHANGED_DURING_TEST"));
        }
        Ok(
            serde_json::json!({"passed":true,"worker_sha256":hash,"host_sha256":host_hash,"capture_rows":capture.rows,"capture_pages":capture.pages,"conversation_count":report.by_conversation.len(),"checkpoint_reopened":true,"report_rows":report.counted_rows,"timeline":timelines,"social_contacts":social.contacts.len(),"social_posts":social.observed_posts,"social_unreadable_posts":social.unreadable_posts,"social_unmatched_login_posts":social.unmatched_login_posts,"social_exports":exports,"exact_locator_verified":located,"source_db_wal_shm_bytes_unchanged":true,"messages_sent":false,"normal_product_state_opened":false,"record_contents_logged":false,"native_gaps_closed":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-social-tools" {
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
        Err(e) => serde_json::json!({"passed":false,"error":e.code,"message":e.message,"messages_sent":false}),
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
