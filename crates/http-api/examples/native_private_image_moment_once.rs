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
        .register_account("自动化测试：私密图片动态一次发布")?
        .id;
    let provider = Arc::new(ManagedWindowsWechatProvider::default());
    let app = Application::new(repository.clone(), provider.clone());
    app.bind_client_local(&account, &clients[0].executable_path)?;
    provider.connect_pinned(&account, host, hash, pid, &clients[0].executable_path)?;
    let result = (|| {
        provider.mark_ready(&account)?;
        let text = "自动化测试：WeRC 双图私密动态顺序与加密持久化验收 2026-11-12。第1张蓝色，第2张绿色，仅自己可见。";
        let mut assets = Vec::new();
        for raw in [
            include_bytes!("../../../tests/fixtures/owned-moment-image-1.png").as_slice(),
            include_bytes!("../../../tests/fixtures/owned-moment-image-2.png").as_slice(),
        ] {
            assets.push(app.upload_image_local(&account, raw.to_vec())?);
        }
        let ids: Vec<_> = assets.iter().map(|a| a.id.clone()).collect();
        let draft = app.create_private_image_moment_local(
            &account,
            text,
            &ids,
            "automation-image-moment-once-20261112",
        )?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if draft.metadata.schema_version != 10
            || preview.payload.text != text
            || preview.payload.moment.is_none()
            || preview.payload.context.target_id != preview.payload.context.sender_id
        {
            return Err(AppError::new("MOMENT_PREVIEW_MISMATCH", "草稿预览不匹配"));
        }
        let image_assets = &preview
            .payload
            .moment
            .as_ref()
            .ok_or_else(AppError::internal)?
            .images;
        if image_assets.iter().map(|a| &a.asset_id).collect::<Vec<_>>()
            != ids.iter().collect::<Vec<_>>()
        {
            return Err(AppError::new("MOMENT_ORDER_MISMATCH", "草稿图片顺序不匹配"));
        }
        let pngs = ids
            .iter()
            .map(|id| app.image_preview_local(&account, id))
            .collect::<AppResult<Vec<_>>>()?;
        let images: Vec<_> = image_assets
            .iter()
            .zip(&pngs)
            .map(|(asset, png)| werc_domain::outbox::MomentImage { asset, png })
            .collect();
        if provider.probe_private_image_moment(text, &images)? != (20, 166) {
            return Err(AppError::new("MOMENT_PROBE_FAILED", "双图只构造核验失败"));
        }
        // Close both handles and verify the persisted draft before approval.
        drop(preview);
        drop(app);
        drop(repository);
        let repository = Arc::new(SqliteRepository::open(&db)?);
        let app = Application::new(repository.clone(), provider.clone());
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        if preview.payload.text != text
            || preview
                .payload
                .moment
                .as_ref()
                .ok_or_else(AppError::internal)?
                .images
                .iter()
                .map(|a| &a.asset_id)
                .collect::<Vec<_>>()
                != ids.iter().collect::<Vec<_>>()
        {
            return Err(AppError::new(
                "MOMENT_REOPEN_MISMATCH",
                "批准前重开内容顺序不匹配",
            ));
        }
        for (id, png) in ids.iter().zip(&pngs) {
            if app.image_preview_local(&account, id)?.as_slice() != png.as_slice() {
                return Err(AppError::internal());
            }
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
        Ok((draft.metadata.id, text.to_owned(), status.state, ids, pngs))
    })();
    let disconnected = provider.disconnect(None);
    disconnected?;
    let (id, text, sent_state, ids, pngs) = result?;
    let repository = Arc::new(SqliteRepository::open(&db)?);
    let reopened = Application::new(repository.clone(), provider);
    let preview = reopened.send_preview_local(&account, &id)?;
    if preview.payload.text != text || preview.payload.moment.is_none() {
        return Err(AppError::new("MOMENT_REOPEN_MISMATCH", "重开内容不匹配"));
    }
    if preview
        .payload
        .moment
        .as_ref()
        .ok_or_else(AppError::internal)?
        .images
        .iter()
        .map(|a| &a.asset_id)
        .collect::<Vec<_>>()
        != ids.iter().collect::<Vec<_>>()
    {
        return Err(AppError::internal());
    }
    for (id, png) in ids.iter().zip(&pngs) {
        if reopened.image_preview_local(&account, id)?.as_slice() != png.as_slice() {
            return Err(AppError::internal());
        }
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
        serde_json::json!({"passed":sent_state==SendState::ServerAccepted&&receipt_saved,"abi":20,"host_sha256":hash,"encrypted_draft_schema":10,"image_count":2,"image_order_preserved":true,"image_bytes_equal_after_reopen":true,"draft_reopened_before_approval":true,"encrypted_store_reopened":true,"exact_plaintext_equal":true,"approved_exact_private_preview":true,"send_attempts":1,"automatic_retry":false,"native_submission_attempted":true,"moments_submitted":true,"server_completed":sent_state==SendState::ServerAccepted,"outbox_state":sent_state,"protected_receipt_saved":receipt_saved,"state_equal_after_reopen":true,"phone_confirmed":false,"local_cache_readback_confirmed":false,"messages_sent":false,"normal_product_state_opened":false,"source_database_opened":false,"complete_product":false}),
    )
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 || args[0] != "--consent-one-private-image-moment-automation-test" {
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
