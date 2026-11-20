//! One authorized private automation video publication. Fixed attempt directory; never replay.
use std::{io::Write,path::Path,sync::Arc};
use werc_application::{Application,ports::Repository};
use werc_domain::{AppError,AppResult,outbox::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{discovery,managed_messages::ManagedWindowsWechatProvider};
fn run(pid:u32,host:&Path,hash:&str,output:&Path)->AppResult<serde_json::Value>{
    let catalog=discovery::discover()?;
    let clients:Vec<_>=catalog.installations.iter().filter(|c|c.process_ids.contains(&pid)).collect();
    if clients.len()!=1||clients[0].version!="4.1.15.13" {return Err(AppError::new("CLIENT_SELECTION_REQUIRED","需精确客户端"))}
    let db=output.join("isolated-state.sqlite3");
    let repository=Arc::new(SqliteRepository::open(&db)?);
    let account=repository.register_account("自动化测试：私密视频动态一次发布")?.id;
    let provider=Arc::new(ManagedWindowsWechatProvider::default());
    let app=Application::new(repository.clone(),provider.clone());
    app.bind_client_local(&account,&clients[0].executable_path)?;
    provider.connect_pinned(&account,host,hash,pid,&clients[0].executable_path)?;
    let result=(||{
        provider.mark_ready(&account)?;
        let video=app.upload_video_local(&account,"自动化测试.mp4",include_bytes!("../../../tests/fixtures/owned-send-video.mp4").to_vec())?;
        let cover=app.upload_image_local(&account,include_bytes!("../../../tests/fixtures/owned-moment-image-1.png").to_vec())?;
        let body="自动化测试：WeRC 原生视频朋友圈验证 2026-11-13。仅自己可见，仅发布一次。";
        let draft=app.create_video_moment_local(&account,body,&video.id,&cover.id,MomentVisibility::Private,"automation-video-moment-once-20261113")?;
        let preview=app.send_preview_local(&account,&draft.metadata.id)?;
        let moment=preview.payload.moment.as_ref().ok_or_else(AppError::internal)?;
        let descriptor=moment.video.as_ref().ok_or_else(AppError::internal)?;
        let png=app.image_preview_local(&account,&cover.id)?;
        let bytes=app.video_contents_local(&account,&video.id)?;
        let saved_descriptor=descriptor.clone();
        if draft.metadata.schema_version!=12 || png.is_empty() || bytes.bytes.is_empty() {
            return Err(AppError::new("VIDEO_PREVIEW_INVALID","批准内容不完整"));
        }
        app.approve_text_send_local(&account,&draft.metadata.id,&draft.metadata.request_digest,true)?;
        let mut marker=std::fs::OpenOptions::new().write(true).create_new(true)
            .open(output.join("submission-started.json")).map_err(|_|AppError::new("ATTEMPT_ALREADY_STARTED","禁止重发"))?;
        marker.write_all(b"{\"attempt_started\":true,\"private_only\":true,\"automatic_retry\":false}").map_err(|_|AppError::internal())?;
        marker.sync_all().map_err(|_|AppError::internal())?;
        let mut status=app.execute_send_local(&account,&draft.metadata.id)?;
        let until=std::time::Instant::now()+std::time::Duration::from_secs(45);
        while status.state==SendState::Unknown && std::time::Instant::now()<until {
            std::thread::sleep(std::time::Duration::from_millis(250));
            status=app.reconcile_send_local(&account,&draft.metadata.id)?;
        }
        Ok((draft.metadata.id,video.id,cover.id,saved_descriptor,body.to_owned(),status.state))
    })();
    let disconnected=provider.disconnect(None);drop(app);drop(repository);disconnected?;
    let (id,video,cover,descriptor,body,sent_state)=result?;
    let app=Application::new(Arc::new(SqliteRepository::open(&db)?),provider);
    let preview=app.send_preview_local(&account,&id)?;
    let moment=preview.payload.moment.as_ref().ok_or_else(AppError::internal)?;
    if preview.status.metadata.schema_version!=12||moment.video.as_ref()!=Some(&descriptor)||preview.payload.text!=body||moment.images[0].asset_id!=cover {
        return Err(AppError::new("VIDEO_MOMENT_REOPEN_MISMATCH","加密重开不一致"));
    }
    if app.video_contents_local(&account,&video)?.bytes.as_slice()!=include_bytes!("../../../tests/fixtures/owned-send-video.mp4") {
        return Err(AppError::new("VIDEO_BYTES_CHANGED","视频副本不一致"));
    }
    if preview.status.state!=sent_state || preview.status.attempt_id.is_none() || app.execute_send_local(&account,&id).is_ok() {
        return Err(AppError::new("VIDEO_ATTEMPT_MISMATCH","一次发布状态不一致"));
    }
    Ok(serde_json::json!({"passed":sent_state==SendState::ServerAccepted,"abi":werc_wechat_win::native_messages::NATIVE_PROTOCOL_VERSION,"outbox_state":sent_state,"schema":12,"encrypted_store_reopened":true,"video_bytes_equal":true,"cover_identity_equal":true,"send_attempts":1,"automatic_retry":false,"moments_submitted":true,"messages_sent":false,"phone_confirmed":false,"normal_product_state_opened":false,"source_database_opened":false,"complete_product":false}))
}
fn main(){
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args.len()!=5||args[0]!="--consent-one-private-video-moment-automation-test"{std::process::exit(2)}
    let output=Path::new(&args[4]);std::fs::create_dir(output).expect("new isolated directory");
    let value=match run(args[1].parse().expect("PID"),Path::new(&args[2]),&args[3],output){
        Ok(v)=>v,Err(e)=>serde_json::json!({"passed":false,"error":e.code,"send_attempt_started":output.join("submission-started.json").exists(),"automatic_retry":false})};
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(output.join("evidence.json")).unwrap();
    file.write_all(&serde_json::to_vec_pretty(&value).unwrap()).unwrap();file.sync_all().unwrap();
    println!("{value}");if value["passed"]!=true{std::process::exit(1)}
}
