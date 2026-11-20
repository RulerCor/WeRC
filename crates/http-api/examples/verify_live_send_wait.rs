//! Real native identity with HTTP draft waiting and cancellation; never sends.
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use std::{io::Write, path::Path, sync::Arc, time::Duration};
use tower::ServiceExt;
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult, outbox::SendState};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{discovery, managed_messages::ManagedWindowsWechatProvider};

fn error(code: &str) -> AppError {
    AppError::new(code, "自动化测试等待验证未通过")
}
async fn run(pid: u32, host: &Path, host_hash: &str) -> AppResult<serde_json::Value> {
    let installations = discovery::discover()?.installations;
    let clients: Vec<_> = installations
        .iter()
        .filter(|i| i.process_ids.contains(&pid))
        .collect();
    if clients.len() != 1 || clients[0].version != "4.1.15.13" {
        return Err(error("CLIENT_SELECTION_REQUIRED"));
    }
    let repo = Arc::new(SqliteRepository::in_memory()?);
    let account = repo
        .register_account("isolated native wait verification")?
        .id;
    let provider = Arc::new(ManagedWindowsWechatProvider::default());
    let app = Arc::new(Application::new(repo, provider.clone()));
    app.bind_client_local(&account, &clients[0].executable_path)?;
    provider.connect_pinned(&account, host, host_hash, pid, &clients[0].executable_path)?;
    let result=async {
        provider.mark_ready(&account)?;
        let token=app.issue_token_local("自动化测试等待",vec![account.clone()],vec!["messages:send".into(),"operations:create".into()],1)?;
        app.enable_api(true);
        let router=werc_http_api::router(app.clone());
        let text="自动化测试：WeRC 等待结果接口草稿验证，仅保存并取消，不发送。";
        let request=|seconds:u16| Request::builder().method("POST").uri("/api/v1/messages/send-text")
            .header("Authorization",format!("Bearer {}",token.secret)).header("Content-Type","application/json")
            .header("Idempotency-Key","automation-native-send-wait-once")
            .body(Body::from(serde_json::json!({"account_id":account,"username":"filehelper","text":text,"wait_seconds":seconds}).to_string())).unwrap();
        for seconds in [0,1] {
            let started=std::time::Instant::now();
            let response=router.clone().oneshot(request(seconds)).await.map_err(|_|error("HTTP_FAILED"))?;
            if response.status()!=200{return Err(error("HTTP_STATUS_FAILED"));}
            let body=response.into_body().collect().await.map_err(|_|error("HTTP_BODY_FAILED"))?.to_bytes();
            let value:serde_json::Value=serde_json::from_slice(&body).map_err(|_|error("HTTP_BODY_FAILED"))?;
            if value["data"]["state"]!="awaiting_approval"||!value["data"]["attempt_id"].is_null() {return Err(error("UNEXPECTED_SUBMISSION"));}
            if seconds==1&&started.elapsed()<Duration::from_millis(900){return Err(error("WAIT_NOT_OBSERVED"));}
        }
        let rows=app.sends_local(&account)?;
        if rows.len()!=1{return Err(error("IDEMPOTENCY_FAILED"));}
        let id=rows[0].metadata.id.clone();
        let preview=app.send_preview_local(&account,&id)?;
        if preview.payload.text!=text||preview.payload.context.target_id!="filehelper"||!preview.payload.context.identity_verified {return Err(error("PREVIEW_FAILED"));}
        drop(preview);
        let pending=tokio::spawn(router.clone().oneshot(request(120)));
        tokio::time::sleep(Duration::from_millis(500)).await;
        if pending.is_finished(){return Err(error("WAIT_RETURNED_EARLY"));}
        app.cancel_send_local(&account,&id)?;
        let response=tokio::time::timeout(Duration::from_secs(5),pending).await.map_err(|_|error("CANCEL_WAIT_TIMEOUT"))?.map_err(|_|error("WAIT_TASK_FAILED"))?.map_err(|_|error("HTTP_FAILED"))?;
        if response.status()!=200{return Err(error("HTTP_STATUS_FAILED"));}
        let body=response.into_body().collect().await.map_err(|_|error("HTTP_BODY_FAILED"))?.to_bytes();
        let value:serde_json::Value=serde_json::from_slice(&body).map_err(|_|error("HTTP_BODY_FAILED"))?;
        if value["data"]["state"]!="canceled"||value["data"]["metadata"]["id"]!=id {return Err(error("CANCEL_RESULT_FAILED"));}
        let rows=app.sends_local(&account)?;
        if rows.len()!=1||rows[0].state!=SendState::Canceled||rows[0].attempt_id.is_some(){return Err(error("UNEXPECTED_SUBMISSION"));}
        app.revoke_token_local(&token.info.id)?;
        app.enable_api(false);
        Ok(serde_json::json!({"passed":true,"abi":17,"host_sha256":host_hash,"real_native_identity_verified":true,"http_router_verified":true,"http_listener_started":false,"wait_seconds_checked":[0,1,120],"one_second_pending_wait_observed":true,"long_wait_interrupted_by_local_cancel":true,"same_idempotent_draft":true,"encrypted_exact_preview_verified":true,"creator_without_operations_read":true,"draft_count":1,"canceled_without_approval":true,"send_attempts":0,"messages_sent":false,"normal_product_state_opened":false,"source_db_accessed":false,"product_store":"isolated_in_memory","records_logged":false,"complete_product":false}))
    }.await;
    provider.disconnect(None)?;
    result
}
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 || args[0] != "--consent-native-wait-no-send" {
        std::process::exit(2);
    }
    let output = Path::new(&args[4]);
    if output.exists() {
        std::process::exit(2);
    }
    std::fs::create_dir(output).unwrap();
    let value = match run(args[1].parse().expect("PID"), Path::new(&args[2]), &args[3]).await {
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
