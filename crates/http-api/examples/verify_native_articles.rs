//! Native publisher history and public body read only; no message or post submission.
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
    output: &Path,
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
        .setup_account_for_source("isolated moments export verification", &fingerprint)?
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
        let paths = vec![path.clone(), root.join("db_storage/contact/contact.db")];
        let before = digests(&paths)?;
        let text = path.to_str().ok_or_else(|| error("SOURCE_PATH"))?;
        app.bind_live_source_local(&account, text, true)?;
        app.acquire_database_key_local(&account, pid, text, true)?;
        let view =
            app.prepare_live_filehelper_local(&account, text, "live-sessions-owned-view", true)?;

        let contacts=app.live_contacts_search_local(&account,&view.view_id,None,Some("gh_"),"article-publisher-discovery")?;
        let mut candidates=contacts.rows.iter().filter(|c|werc_domain::article_history::publisher_valid(&c.anchor.username)&&c.delete_flag.unwrap_or(0)==0).collect::<Vec<_>>();
        candidates.sort_by_key(|c|if c.nickname.as_deref().is_some_and(|n|n.contains("人民日报")||n.contains("微信公众平台")||n.contains("腾讯新闻")){0}else{1});
        let mut rejected=Vec::new();let mut selected=None;
        for publisher in candidates.into_iter().take(5){
            match app.article_action_local(&account,werc_application::articles::ArticleAction::History{publisher:publisher.anchor.username.clone(),cursor:None},"article-native-history-first") {
                Ok(first)=>{let page:werc_domain::article_history::ArticleHistoryPage=serde_json::from_value(first).map_err(|_|error("HISTORY_FORMAT"))?;selected=Some((publisher,page));break;},
                Err(e) if e.code=="ARTICLE_HISTORY_SERVER_ERROR"=>rejected.push(e.message),Err(e)=>return Err(e),
            }
        }
        let (publisher,page)=selected.ok_or_else(||AppError::new("NO_READABLE_PUBLISHER_SAMPLE",&format!("独立样本返回：{rejected:?}")))?;
        let second=if let Some(cursor)=&page.next_cursor{Some(app.article_action_local(&account,werc_application::articles::ArticleAction::History{publisher:publisher.anchor.username.clone(),cursor:Some(cursor.clone())},"article-native-history-next")?)}else{None};
        let mut fetched=0;let mut fetch_error=None;let mut exported=Vec::new();let mut body_mode=None;let mut cached_images=0;let mut image_errors=Vec::new();
        if let Some(article)=page.data.first(){
            match app.fetch_article_local(&account,&article.url,"article-real-public-body") {
                Ok(preview)=>{
                    let saved=app.confirm_article_local(&account,&preview.id,preview.revision,true)?;
                    fetched+=1;body_mode=saved.record.saved.as_ref().map(|a|a.source_mode.clone());
                    for index in 0..saved.record.saved.as_ref().map_or(0,|a|a.images.len()).min(3){match app.cache_article_image_local(&account,&saved.id,index,"article-real-image"){Ok(m)=>{let (_,b)=app.article_image_contents_local(&account,&m.id)?;if b.len()!=m.size{return Err(error("ARTICLE_IMAGE_READBACK"));}cached_images+=1;},Err(e)=>image_errors.push(e.code)}}
                    for format in [werc_application::articles::ArticleExportFormat::Json,werc_application::articles::ArticleExportFormat::Md,werc_application::articles::ArticleExportFormat::Html,werc_application::articles::ArticleExportFormat::Xlsx,werc_application::articles::ArticleExportFormat::Zip]{
                        let out=app.export_articles_local(&account,&[saved.id.clone()],format,"article-real-format")?;
                        // Verification artifacts contain only this selected public article.
                        std::fs::write(output.join(format!("public-article.{}",format.extension())),&out.bytes).map_err(|_|error("OUTPUT"))?;
                        exported.push(serde_json::json!({"format":format.extension(),"bytes":out.bytes.len(),"sha256":hex::encode(Sha256::digest(&out.bytes))}));
                    }
                },Err(e)=>fetch_error=Some(e.code),
            }
        }
        let after=digests(&paths)?;
        Ok(serde_json::json!({"passed":before==after && fetched>0 && fetch_error.is_none() && image_errors.is_empty(),"native_history_passed":true,"public_body_passed":fetched>0 && fetch_error.is_none(),"abi":26,"native_first_rows":page.data.len(),"unavailable_sample_count":rejected.len(),"native_second_rows":second.as_ref().and_then(|v|v["data"].as_array()).map(Vec::len),"end_reached":page.end_reached,"skipped":page.skipped,"body_mode":body_mode,"cached_images":cached_images,"image_errors":image_errors,"public_bodies_saved":fetched,"public_body_error":fetch_error,"exports":exported,"source_db_wal_shm_bytes_unchanged":before==after,"messages_sent":false,"normal_product_state_opened":false,"complete_product":false}))

    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-article-history" {
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
        Err(e) => serde_json::json!({"passed":false,"error":e.code,"detail":e.message,"messages_sent":false}),
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
