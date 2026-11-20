//! Explicitly authorized construction/cleanup and encrypted-draft verification.
//! There is no approval or native submission operation in this executable.
use std::{io::Write, path::Path, sync::Arc};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult, outbox::SendState};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{discovery, managed_messages::ManagedWindowsWechatProvider};
fn run(
    pid: u32,
    host: &Path,
    hash: &str,
    sns_id: &str,
    output: &Path,
) -> AppResult<serde_json::Value> {
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
        let (code, stage) = provider.probe_moment_action(sns_id)?;
        if (code, stage) != (22, 5) {
            return Err(AppError::new(
                "MOMENT_LOOKUP_FAILED",
                &format!("code={code},stage={stage}"),
            ));
        }
        let boundaries = vec![code, stage];
        let text = "自动化测试：评论草稿重开验证，不提交。";
        let draft = app.create_moment_action_local(
            &account,
            &werc_domain::outbox::MomentAction {
                reply: None,
                sns_id: sns_id.into(),
                action: werc_domain::outbox::MomentActionKind::Comment,
            },
            text,
            "automation-moment-action-probe-001",
        )?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if draft.metadata.schema_version != 11
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
        serde_json::json!({"passed":true,"abi":22,"host_sha256":hash,"lookup_receipt_state":boundaries,"encrypted_draft_schema":11,"encrypted_store_reopened":true,"exact_plaintext_equal":true,"draft_canceled_without_approval":true,"native_submission_attempted":false,"moments_submitted":false,"messages_sent":false,"normal_product_state_opened":false,"source_database_opened":false,"complete_product":false}),
    )
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 || args[0] != "--consent-moment-lookup-no-submit" {
        std::process::exit(2)
    }
    let output = Path::new(&args[5]);
    std::fs::create_dir(output).expect("new isolated directory");
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        &args[4],
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
