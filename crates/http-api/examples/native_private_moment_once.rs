//! One authorized private automation test through the encrypted outbox. Never replay.
use std::{io::Write, path::Path, sync::Arc};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult, outbox::SendState};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{discovery, managed_messages::ManagedWindowsWechatProvider};
fn run(pid: u32, host: &Path, hash: &str, state: &Path) -> AppResult<serde_json::Value> {
    let catalog = discovery::discover()?;
    let clients: Vec<_> = catalog
        .installations
        .iter()
        .filter(|c| c.process_ids.contains(&pid))
        .collect();
    if clients.len() != 1 || clients[0].version != "4.1.15.13" {
        return Err(AppError::new("CLIENT_SELECTION_REQUIRED", "需要精确客户端"));
    }
    let db = state.join("isolated-state.sqlite3");
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
        let text = "自动化测试：WeRC 正式加密队列私密动态验收 2026-11-12。仅自己可见。";
        if provider.probe_private_moment(text)? != (20, 166) {
            return Err(AppError::new("MOMENT_PROBE_FAILED", "只构造核验失败"));
        }
        let draft = app.create_private_moment_local(
            &account,
            text,
            "automation-private-moment-formal-once-20261112",
        )?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if draft.metadata.schema_version != 9
            || preview.payload.text != text
            || preview.payload.moment.is_none()
            || preview.payload.context.target_id != preview.payload.context.sender_id
        {
            return Err(AppError::new("MOMENT_PREVIEW_MISMATCH", "草稿预览不匹配"));
        }
        app.approve_text_send_local(
            &account,
            &draft.metadata.id,
            &draft.metadata.request_digest,
            true,
        )?;
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(state.join("submission-started.json"))
            .map_err(|_| AppError::new("ATTEMPT_ALREADY_STARTED", "禁止重发"))?;
        marker
            .write_all(
                b"{\"attempt_started\":true,\"automatic_retry\":false,\"private_only\":true}",
            )
            .map_err(|_| AppError::internal())?;
        marker.sync_all().map_err(|_| AppError::internal())?;
        let mut status = app.execute_send_local(&account, &draft.metadata.id)?;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(40);
        while status.state == SendState::Unknown && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(250));
            status = app.reconcile_send_local(&account, &draft.metadata.id)?;
        }
        Ok((draft.metadata.id, text.to_owned(), status.state))
    })();
    let disconnected = provider.disconnect(None);
    drop(app);
    drop(repository);
    disconnected?;
    let (id, text, sent_state) = result?;
    let repository = Arc::new(SqliteRepository::open(&db)?);
    let reopened = Application::new(repository.clone(), provider);
    let preview = reopened.send_preview_local(&account, &id)?;
    if preview.payload.text != text || preview.payload.moment.is_none() {
        return Err(AppError::new("MOMENT_REOPEN_MISMATCH", "重开内容不匹配"));
    }
    let persisted = repository.send_status(&account, &id)?;
    let receipt_saved = repository.send_receipt(&account, &id)?.is_some();
    if persisted.state != sent_state || persisted.attempt_id.is_none() {
        return Err(AppError::new(
            "MOMENT_REOPEN_STATE_MISMATCH",
            "重开状态不匹配",
        ));
    }
    Ok(
        serde_json::json!({"passed":sent_state==SendState::ServerAccepted&&receipt_saved,"abi":19,"host_sha256":hash,"encrypted_draft_schema":9,"encrypted_store_reopened":true,"exact_plaintext_equal":true,"approved_exact_private_preview":true,"send_attempts":1,"automatic_retry":false,"native_submission_attempted":true,"moments_submitted":true,"server_completed":sent_state==SendState::ServerAccepted,"outbox_state":sent_state,"protected_receipt_saved":receipt_saved,"state_equal_after_reopen":true,"phone_confirmed":false,"local_cache_readback_confirmed":false,"messages_sent":false,"normal_product_state_opened":false,"source_database_opened":false,"complete_product":false}),
    )
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 || args[0] != "--consent-one-private-moment-automation-test" {
        std::process::exit(2)
    }
    let state = Path::new(&args[5]);
    std::fs::create_dir(state).expect("fixed attempt already exists; never resend");
    let output = Path::new(&args[4]);
    std::fs::create_dir(output).expect("new evidence directory");
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        state,
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"send_attempt_started":state.join("submission-started.json").exists(),"automatic_retry":false})
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
