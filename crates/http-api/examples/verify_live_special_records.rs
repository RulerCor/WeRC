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
        .setup_account_for_source("isolated special records export verification", &fingerprint)?
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
        let paths = vec![path.clone(), root.join("db_storage/general/general.db")];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;

        use werc_domain::special_records::{RecordQuery, RecordSource};
        let started = std::time::Instant::now();
        let mut reports = Vec::new();
        for kind in [
            RecordSource::FriendRequest,
            RecordSource::SearchChat,
            RecordSource::SearchWeb,
            RecordSource::SearchBrand,
            RecordSource::Revoke,
            RecordSource::Transfer,
            RecordSource::RedPacket,
        ] {
            let selection = RecordQuery {
                source: kind,
                text: String::new(),
                from: None,
                until: None,
            };
            // Renew the authenticated view per independent tool, not per export page.
            let current =
                app.prepare_live_filehelper_local(&account, text, "live-records-owned-view", true)?;
            let page = app.live_special_records_local(
                &account,
                &current.view_id,
                Some(selection.clone()),
                None,
                &uuid::Uuid::new_v4().to_string(),
            )?;
            let first_count = page.rows.len();
            let report=app.export_special_records_local(&account,&current.view_id,selection,&uuid::Uuid::new_v4().to_string(),"json",|bytes,count|{
                let parsed:serde_json::Value=serde_json::from_slice(bytes).map_err(|_|error("EXPORT_JSON_INVALID"))?;
                let rows=parsed["rows"].as_array().ok_or_else(||error("EXPORT_ROWS_INVALID"))?;
                if rows.len()!=count||count<first_count{return Err(error("EXPORT_COUNT_MISMATCH"));}
                let mut ids=std::collections::BTreeSet::new();
                for row in rows{if !ids.insert(row["anchor"]["row_id"].as_str().ok_or_else(||error("ROW_ID_INVALID"))?){return Err(error("DUPLICATE_RECORD"));}}
                Ok(serde_json::json!({"source":kind,"first_page":first_count,"rows":count,"dated":rows.iter().filter(|r|r["timestamp"].is_i64()).count(),"visible_text":rows.iter().filter(|r|r["text"].as_str().is_some_and(|s|!s.is_empty())).count(),"bytes":bytes.len(),"json_sha256":hex::encode(Sha256::digest(bytes))}))
            })?;
            reports.push(report);
        }
        let unchanged = before == digests(&paths)?;
        Ok(
            serde_json::json!({"passed":unchanged,"reports":reports,"elapsed_ms":started.elapsed().as_millis(),"source_db_wal_shm_bytes_unchanged":unchanged,"worker_sha256":hash,"host_sha256":host_hash,"messages_sent":false,"normal_product_state_opened":false,"records_persisted":false,"complete_product":false}),
        )
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-special-records" {
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
