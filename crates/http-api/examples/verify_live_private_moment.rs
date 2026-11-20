//! Explicitly authorized construction/cleanup and encrypted-draft verification.
//! There is no approval or native submission operation in this executable.
use std::{io::Write, path::Path, sync::Arc};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult, outbox::SendState};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{discovery, managed_messages::ManagedWindowsWechatProvider};
fn run(pid: u32, host: &Path, hash: &str, output: &Path) -> AppResult<serde_json::Value> {
    let catalog = discovery::discover()?;
    let clients: Vec<_> = catalog
        .installations
        .iter()
        .filter(|c| c.process_ids.contains(&pid))
        .collect();
    if clients.len() != 1 || clients[0].version != "4.1.15.13" {
        return Err(AppError::new("CLIENT_SELECTION_REQUIRED", "需要精确客户端"));
    }
    let db = output.join("isolated-state.sqlite3");
    let repository = Arc::new(SqliteRepository::open(&db)?);
    let account = repository
        .register_account("自动化测试：私密动态只构造")?
        .id;
    let provider = Arc::new(ManagedWindowsWechatProvider::default());
    let app = Application::new(repository.clone(), provider.clone());
    app.bind_client_local(&account, &clients[0].executable_path)?;
    provider.connect_pinned(&account, host, hash, pid, &clients[0].executable_path)?;
    let result = (|| {
        provider.mark_ready(&account)?;
        let mut boundaries = Vec::new();
        for count in [1, 15, 16, 1023, 1024, 4095, 4096, 10000] {
            let text = "中".repeat(count / 3) + &"x".repeat(count % 3);
            let (code, stage) = provider.probe_private_moment(&text)?;
            if (code, stage) != (20, 166) {
                return Err(AppError::new(
                    "MOMENT_CONSTRUCTION_FAILED",
                    &format!("边界{count} code{code} stage{stage}"),
                ));
            }
            boundaries.push(count);
        }
        if provider.probe_private_moment(&"x".repeat(10001)).is_ok() {
            return Err(AppError::new("MOMENT_LIMIT_NOT_ENFORCED", "超限未拒绝"));
        }
        let text = "自动化测试：私密动态正式加密草稿验证，只保存和取消，不发布。";
        let draft =
            app.create_private_moment_local(&account, text, "automation-private-moment-probe-001")?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if draft.metadata.schema_version != 9
            || preview.payload.text != text
            || preview.payload.moment.is_none()
            || preview.payload.context.target_id != preview.payload.context.sender_id
        {
            return Err(AppError::new("MOMENT_PREVIEW_MISMATCH", "草稿预览不匹配"));
        }
        Ok((draft.metadata.id, text.to_owned(), boundaries))
    })();
    let disconnected = provider.disconnect(None);
    drop(app);
    drop(repository);
    disconnected?;
    let (id, text, boundaries) = result?;
    let repository = Arc::new(SqliteRepository::open(&db)?);
    let reopened = Application::new(repository.clone(), provider);
    let preview = reopened.send_preview_local(&account, &id)?;
    if preview.payload.text != text || preview.payload.moment.is_none() {
        return Err(AppError::new("MOMENT_REOPEN_MISMATCH", "重开内容不匹配"));
    }
    let canceled = reopened.cancel_send_local(&account, &id)?;
    if canceled.state != SendState::Canceled || canceled.attempt_id.is_some() {
        return Err(AppError::new("MOMENT_UNEXPECTED_ATTEMPT", "不应产生提交"));
    }
    Ok(
        serde_json::json!({"passed":true,"abi":20,"host_sha256":hash,"utf8_boundaries":boundaries,"probe_receipt":20,"native_stage":166,"private_model_fields_checked":true,"completion_event_connected_canceled":true,"encrypted_draft_schema":9,"encrypted_store_reopened":true,"exact_plaintext_equal":true,"draft_canceled_without_approval":true,"native_submission_attempted":false,"moments_submitted":false,"messages_sent":false,"normal_product_state_opened":false,"source_database_opened":false,"complete_product":false}),
    )
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 5 || args[0] != "--consent-private-moment-construction-no-send" {
        std::process::exit(2)
    }
    let output = Path::new(&args[4]);
    std::fs::create_dir(output).expect("new isolated directory");
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        output,
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"messages_sent":false,"moments_submitted":false})
        }
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
        std::process::exit(1)
    }
}
