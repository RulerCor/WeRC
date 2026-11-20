//! Native video model construction and encrypted reopening. Never publishes.
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
    let account=repository.register_account("自动化测试：朋友圈视频构造")?.id;
    let provider=Arc::new(ManagedWindowsWechatProvider::default());
    let app=Application::new(repository.clone(),provider.clone());
    app.bind_client_local(&account,&clients[0].executable_path)?;
    provider.connect_pinned(&account,host,hash,pid,&clients[0].executable_path)?;
    let result=(||{
        provider.mark_ready(&account)?;
        let video=app.upload_video_local(&account,"自动化测试.mp4",include_bytes!("../../../tests/fixtures/owned-send-video.mp4").to_vec())?;
        let cover=app.upload_image_local(&account,include_bytes!("../../../tests/fixtures/owned-image.png").to_vec())?;
        let body="自动化测试：朋友圈原生视频，只构造不发布。";
        let draft=app.create_video_moment_local(&account,body,&video.id,&cover.id,MomentVisibility::Private,"automation-video-moment-probe-01")?;
        let preview=app.send_preview_local(&account,&draft.metadata.id)?;
        let moment=preview.payload.moment.as_ref().ok_or_else(AppError::internal)?;
        let descriptor=moment.video.as_ref().ok_or_else(AppError::internal)?;
        let png=app.image_preview_local(&account,&cover.id)?;
        let bytes=app.video_contents_local(&account,&video.id)?;
        let (receipt,stage)=provider.probe_video_moment(body,MomentImage{asset:&moment.images[0],png:&png},descriptor,&bytes.bytes,MomentVisibility::Private)?;
        if receipt!=20||stage!=166{return Err(AppError::new(&format!("MOMENT_VIDEO_PROBE_{receipt}_{stage}"),"视频模型构造未通过"))}
        Ok((draft.metadata.id,video.id,cover.id,descriptor.clone(),body.to_owned()))
    })();
    let disconnected=provider.disconnect(None);drop(app);drop(repository);disconnected?;
    let (id,video,cover,descriptor,body)=result?;
    let app=Application::new(Arc::new(SqliteRepository::open(&db)?),provider);
    let preview=app.send_preview_local(&account,&id)?;
    let moment=preview.payload.moment.as_ref().ok_or_else(AppError::internal)?;
    if preview.status.metadata.schema_version!=12||moment.video.as_ref()!=Some(&descriptor)||preview.payload.text!=body||moment.images[0].asset_id!=cover {
        return Err(AppError::new("VIDEO_MOMENT_REOPEN_MISMATCH","加密重开不一致"));
    }
    if app.video_contents_local(&account,&video)?.bytes.as_slice()!=include_bytes!("../../../tests/fixtures/owned-send-video.mp4") {
        return Err(AppError::new("VIDEO_BYTES_CHANGED","视频副本不一致"));
    }
    let status=app.cancel_send_local(&account,&id)?;
    if status.attempt_id.is_some()||status.state!=SendState::Canceled{return Err(AppError::internal())}
    Ok(serde_json::json!({"passed":true,"abi":werc_wechat_win::native_messages::NATIVE_PROTOCOL_VERSION,"probe_receipt":20,"native_stage":166,"schema":12,"encrypted_store_reopened":true,"video_bytes_equal":true,"cover_identity_equal":true,"draft_canceled_without_attempt":true,"moments_submitted":false,"messages_sent":false,"normal_product_state_opened":false,"source_database_opened":false,"complete_product":false}))
}
fn main(){
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args.len()!=5||args[0]!="--consent-video-moment-construction-no-send"{std::process::exit(2)}
    let output=Path::new(&args[4]);std::fs::create_dir(output).expect("new isolated directory");
    let value=match run(args[1].parse().expect("PID"),Path::new(&args[2]),&args[3],output){
        Ok(v)=>v,Err(e)=>serde_json::json!({"passed":false,"error":e.code,"moments_submitted":false,"messages_sent":false})};
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(output.join("evidence.json")).unwrap();
    file.write_all(&serde_json::to_vec_pretty(&value).unwrap()).unwrap();file.sync_all().unwrap();
    println!("{value}");if value["passed"]!=true{std::process::exit(1)}
}
