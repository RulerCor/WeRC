//! Formal provider model construction plus encrypted ordered image draft reopening.
//! No approval or publication operation is reachable in this executable.
use std::{io::Write, path::Path, sync::Arc};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult, outbox::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{discovery, managed_messages::ManagedWindowsWechatProvider};

fn run(
    pid: u32,
    host: &Path,
    hash: &str,
    output: &Path,
    input: &Path,
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
        .register_account("自动化测试：私密图片动态只构造")?
        .id;
    let provider = Arc::new(ManagedWindowsWechatProvider::default());
    let app = Application::new(repository.clone(), provider.clone());
    app.bind_client_local(&account, &clients[0].executable_path)?;
    provider.connect_pinned(&account, host, hash, pid, &clients[0].executable_path)?;
    let result = (|| {
        provider.mark_ready(&account)?;
        let raw = std::fs::read(input).map_err(|_| AppError::internal())?;
        let jpeg_input = raw.starts_with(&[255, 216]);
        let first = app.upload_image_local(&account, raw.clone())?;
        if jpeg_input && (first.image.width, first.image.height) != (640, 360) {
            return Err(AppError::new(
                "JPEG_ORIENTATION_MISMATCH",
                "测试照片应按方向转正",
            ));
        }
        let second = app.upload_image_local(&account, raw)?;
        let chat = app.create_image_send_local(
            &account,
            "filehelper",
            &first.id,
            "automation-jpeg-chat-draft-01",
        )?;
        if chat.metadata.schema_version != 3
            || app
                .send_preview_local(&account, &chat.metadata.id)?
                .payload
                .image
                .as_ref()
                .is_none_or(|image| {
                    image.width != first.image.width
                        || image.height != first.image.height
                        || image.png_sha256 != first.sha256
                        || image.source_sha256 != first.source_sha256
                })
        {
            return Err(AppError::new("IMAGE_DRAFT_MISMATCH", "图片草稿不匹配"));
        }
        let canceled = app.cancel_send_local(&account, &chat.metadata.id)?;
        if canceled.state != SendState::Canceled || canceled.attempt_id.is_some() {
            return Err(AppError::internal());
        }
        let ids = vec![second.id.clone(), first.id.clone()];
        let body = "自动化测试：WeRC 图片朋友圈加密草稿和顺序核验，只构造不发布。";
        let draft = app.create_private_image_moment_local(
            &account,
            body,
            &ids,
            "automation-image-moment-probe-01",
        )?;
        let preview = app.send_preview_local(&account, &draft.metadata.id)?;
        let assets = &preview
            .payload
            .moment
            .as_ref()
            .ok_or_else(AppError::internal)?
            .images;
        if draft.metadata.schema_version != 10
            || assets.iter().map(|a| &a.asset_id).collect::<Vec<_>>()
                != ids.iter().collect::<Vec<_>>()
            || preview.payload.text != body
        {
            return Err(AppError::new(
                "MOMENT_PREVIEW_MISMATCH",
                "草稿内容顺序不匹配",
            ));
        }
        let png = app.image_preview_local(&account, &first.id)?;
        let mut boundaries = Vec::new();
        for count in [1, 2, 9] {
            let text = match count {
                1 => String::new(),
                2 => body.to_owned(),
                _ => "中".repeat(3333) + "x",
            };
            let images: Vec<_> = (0..count)
                .map(|_| MomentImage {
                    asset: &assets[0],
                    png: &png,
                })
                .collect();
            if provider.probe_private_image_moment(&text, &images)? != (20, 166) {
                return Err(AppError::new(
                    "MOMENT_IMAGE_CONSTRUCTION_FAILED",
                    "正式图片模型核验失败",
                ));
            }
            boundaries.push(serde_json::json!({"images":count,"utf8_bytes":text.len()}));
        }
        Ok((
            draft.metadata.id,
            body.to_owned(),
            ids,
            png,
            boundaries,
            jpeg_input,
        ))
    })();
    let disconnected = provider.disconnect(None);
    drop(app);
    drop(repository);
    disconnected?;
    let (id, body, ids, png, boundaries, jpeg_input) = result?;
    let repository = Arc::new(SqliteRepository::open(&db)?);
    let app = Application::new(repository, provider);
    let preview = app.send_preview_local(&account, &id)?;
    let assets = &preview
        .payload
        .moment
        .as_ref()
        .ok_or_else(AppError::internal)?
        .images;
    if preview.payload.text != body
        || assets.iter().map(|a| &a.asset_id).collect::<Vec<_>>() != ids.iter().collect::<Vec<_>>()
    {
        return Err(AppError::new(
            "MOMENT_REOPEN_MISMATCH",
            "重开内容顺序不匹配",
        ));
    }
    for image in &ids {
        if app.image_preview_local(&account, image)?.as_slice() != png.as_slice() {
            return Err(AppError::new(
                "MOMENT_IMAGE_REOPEN_MISMATCH",
                "重开图片不匹配",
            ));
        }
    }
    let status = app.cancel_send_local(&account, &id)?;
    if status.state != SendState::Canceled || status.attempt_id.is_some() {
        return Err(AppError::new("MOMENT_UNEXPECTED_ATTEMPT", "不应产生提交"));
    }
    for image in ids {
        app.delete_image_local(&account, &image)?;
    }
    if !app.image_assets_local(&account)?.is_empty() {
        return Err(AppError::internal());
    }
    Ok(
        serde_json::json!({"passed":true,"abi":20,"host_sha256":hash,"boundaries":boundaries,"probe_receipt":20,"native_stage":166,"encrypted_draft_schema":10,"encrypted_store_reopened":true,"exact_plaintext_equal":true,"image_order_preserved":true,"image_bytes_equal_after_reopen":true,"image_assets_deleted":true,"draft_canceled_without_approval":true,"jpeg_input":jpeg_input,"jpeg_orientation_verified":jpeg_input,"chat_image_draft_preview_cancel_verified":true,"native_submission_attempted":false,"moments_submitted":false,"messages_sent":false,"normal_product_state_opened":false,"source_database_opened":false,"complete_product":false}),
    )
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 || args[0] != "--consent-private-image-moment-construction-no-send" {
        std::process::exit(2)
    }
    let output = Path::new(&args[4]);
    std::fs::create_dir(output).expect("new isolated directory");
    let value = match run(
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        output,
        Path::new(&args[5]),
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
