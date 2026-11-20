//! Offline verification of the encrypted real capture produced by verify_live_social_tools.
//! Opens only the explicitly named isolated test state; never attaches or sends.
use std::{path::Path,sync::Arc};
use sha2::{Digest,Sha256};
use werc_application::{Application, ports::Repository};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;
use werc_domain::{AppError,AppResult};
fn error(code:&str)->AppError {AppError::new(code,"已保存真实资料验证未通过")}
fn run(path:&Path)->AppResult<serde_json::Value>{
 let repo=Arc::new(SqliteRepository::open(&path.join("state.sqlite3"))?);
 let accounts=repo.accounts()?;
 if accounts.len()!=1{return Err(error("ACCOUNT_SELECTION_REQUIRED"));}
 let account=accounts[0].id.clone();
 let app=Application::new(repo.clone(),Arc::new(WindowsWechatProvider));
 let roots=repo.library_list(&account,"source_capture",None,100)?;
 if roots.data.len()!=1{return Err(error("CAPTURE_SELECTION_REQUIRED"));}
 let capture=app.source_capture_status_local(&account,&roots.data[0].id)?;
 if !capture.complete_selected_sources || capture.rows==0{return Err(error("CAPTURE_INCOMPLETE"));}
        let report = app.capture_report_local(
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
            let timeline=app.capture_timeline_local(&account,&capture.id,&query,None,"real-social-timeline")?;
            let mut next_rows=0;
            if let Some(cursor)=&timeline.next {
                let next=app.capture_timeline_local(&account,&capture.id,&query,Some(cursor),"real-social-timeline-next")?;
                let last=timeline.data.last().ok_or_else(||error("TIMELINE_CURSOR_WITHOUT_ROWS"))?;
                if next.data.iter().any(|r|(r.timestamp,r.page,r.row)>=(last.timestamp,last.page,last.row)){return Err(error("TIMELINE_ORDER"));}
                next_rows=next.data.len();
            }
            timelines.push(serde_json::json!({"mode":mode,"first_page":timeline.data.len(),"next_page":next_rows,"scanned":timeline.scanned,"mentions":timeline.summary.mentions}));
        }
        let filter=werc_application::capture_tools::CaptureFilter::default();
        let social=app.capture_social_report_local(&account,&capture.id,&filter,"real-social-report")?;
        if social.unreadable_posts!=0 || social.unmatched_login_posts!=0 || social.observed_posts==0 {return Err(error("SOCIAL_RECORDS_INCOMPLETE"));}
        let mut exports=Vec::new();
        for format in ["json","txt","html"] {
            let bytes=zeroize::Zeroizing::new(app.export_capture_social_local(&account,&capture.id,&filter,format,"real-social-export")?);
            if bytes.is_empty(){return Err(error("SOCIAL_EXPORT_EMPTY"));}
            if format=="json" {let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|_|error("SOCIAL_EXPORT_INVALID"))?;if value["contacts"].as_array().map(Vec::len)!=Some(social.contacts.len()){return Err(error("SOCIAL_EXPORT_COUNT"));}}
            exports.push(serde_json::json!({"format":format,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(&*bytes))}));
        }
        let mut located=false;
        for index in 0..capture.pages {
            let page=app.source_capture_page_local(&account,&capture.id,index)?;
            for row in &page.rows {
                if let Some(id)=row.message.server_id.as_deref().filter(|s|*s!="0") {
                    let found=app.capture_message_location_local(&account,&capture.id,&row.username,id,"real-social-locate")?;
                    let target=app.source_capture_page_local(&account,&capture.id,found.position.page)?;
                    let matched=target.rows.get(found.position.row).ok_or_else(||error("LOCATOR_ROW_MISSING"))?;
                    if matched.username!=row.username || matched.message.server_id.as_deref()!=Some(id) {return Err(error("LOCATOR_IDENTITY_MISMATCH"));}
                    located=true;break;
                }
            }
            if located {break;}
        }
        if !located{return Err(error("LOCATOR_SAMPLE_MISSING"));}

 Ok(serde_json::json!({"passed":true,"capture_rows":capture.rows,"capture_pages":capture.pages,"report_rows":report.counted_rows,"conversation_count":report.by_conversation.len(),"timeline":timelines,"social_contacts":social.contacts.len(),"social_posts":social.observed_posts,"social_exports":exports,"exact_locator_verified":located,"isolated_real_capture_reopened":true,"live_sources_opened":false,"native_connected":false,"messages_sent":false,"normal_product_state_opened":false,"live_source_invariance_claimed":false,"complete_product":false}))
}
fn main(){let args:Vec<_>=std::env::args().skip(1).collect();if args.len()!=1{std::process::exit(2);}
 let path=Path::new(&args[0]);let result=run(path);let value=match result {Ok(v)=>v,Err(e)=>serde_json::json!({"passed":false,"error":e.code,"message":e.message})};
 use std::io::Write;let mut f=std::fs::OpenOptions::new().write(true).create_new(true).open(path.join("offline-validation.json")).expect("new validation evidence");f.write_all(&serde_json::to_vec_pretty(&value).unwrap()).unwrap();f.sync_all().unwrap();println!("{value}");if value["passed"]!=true {std::process::exit(1);}}
