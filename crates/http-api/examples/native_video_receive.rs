//! Receive-only research from the isolated encrypted test observations.
//! No send operation, normal product store, decrypted file or credential log.
#![cfg(windows)]
#[path = "support/client.rs"]
mod client;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppError, AppResult, Principal};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{WindowsWechatProvider, managed_messages::ManagedWindowsWechatProvider};

fn decode(executable: &Path, bytes: &[u8]) -> AppResult<serde_json::Value> {
    let mut child = Command::new(executable)
        .args([
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-xerror",
            "-err_detect",
            "explode",
            "-threads",
            "2",
            "-max_pixels",
            "16777216",
            "-protocol_whitelist",
            "pipe",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-threads",
            "2",
            "-progress",
            "pipe:1",
            "-f",
            "null",
            "-",
        ])
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            AppError::new(
                "VIDEO_DECODER_START",
                "verification decoder could not start",
            )
        })?;
    let mut input = child.stdin.take().ok_or_else(AppError::internal)?;
    let output = child.stdout.take().ok_or_else(AppError::internal)?;
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || input.write_all(bytes));
        let reader = scope.spawn(|| {
            let mut result = String::new();
            output
                .take(65537)
                .read_to_string(&mut result)
                .map(|_| result)
        });
        let until = Instant::now() + Duration::from_secs(45);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        // Close stdin in the writer even after a failed decode; never wait for
        // the decoder's EOF with an open writer retained by this scope.
        let written = writer.join().map_err(|_| AppError::internal())?;
        let report = reader
            .join()
            .map_err(|_| AppError::internal())?
            .map_err(|_| AppError::internal())?;
        if !status.is_some_and(|s| s.success())
            || written.is_err()
            || report.len() > 65536
            || !report.contains("progress=end")
        {
            return Err(AppError::new(
                "VIDEO_DECODE_FAILED",
                "bounded full decode did not complete",
            ));
        }
        let frames = report
            .lines()
            .filter_map(|l| l.strip_prefix("frame="))
            .filter_map(|v| v.trim().parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        let duration = report
            .lines()
            .filter_map(|l| l.strip_prefix("out_time_us="))
            .filter_map(|v| v.trim().parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        if frames == 0 || duration == 0 {
            return Err(AppError::new("VIDEO_DECODE_EMPTY", "no decoded frames"));
        }
        Ok(
            serde_json::json!({"full_decode_completed":true,"frames":frames,"duration_us":duration,"verification_decoder_only":true}),
        )
    })
}

fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 7 {
        return Err(AppError::new(
            "ARGUMENTS",
            "host hash pid fresh-directory isolated-snapshot decoder",
        ));
    }
    let path = PathBuf::from(&a[4]);
    std::fs::create_dir(&path)
        .map_err(|_| AppError::new("STATE_EXISTS", "fresh directory required"))?;
    let db = path.join("state.sqlite3");
    std::fs::copy(&a[5], &db).map_err(|_| AppError::internal())?;
    let repo = Arc::new(SqliteRepository::open(&db)?);
    let accounts = repo.accounts()?;
    if accounts.len() != 1 {
        return Err(AppError::internal());
    }
    let account = accounts[0].id.clone();
    let provider = Arc::new(ManagedWindowsWechatProvider::default());
    let app = Application::new(repo.clone(), provider.clone());
    let rows = app.received_texts_local(&account, 0, 100)?;
    let row = rows
        .iter()
        .find(|r| r.payload.message.message_type == 43 && r.payload.target_id == "filehelper")
        .ok_or_else(AppError::internal)?;
    let descriptor = match werc_domain::received_media::video_descriptor(&row.payload.message) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            let mut summary = serde_json::json!({"error_code":error.code,"native_download_attempted":false,"valid_content":row.payload.message.valid_content()});
            let mut reader = quick_xml::Reader::from_str(
                row.payload.message.metadata.as_deref().unwrap_or_default(),
            );
            for _ in 0..2048 {
                match reader.read_event() {
                    Ok(quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e))
                        if e.name().as_ref() == "videomsg" =>
                    {
                        for a in e.attributes().flatten() {
                            match a.key.as_ref() {
                                "length" => {
                                    summary["declared_size"] = a.value.parse::<u64>().ok().into()
                                }
                                "cdnvideourl" => {
                                    summary["file_key_length"] = a.value.len().into();
                                    summary["file_key_hex"] =
                                        a.value.bytes().all(|b| b.is_ascii_hexdigit()).into();
                                }
                                "aeskey" => {
                                    summary["aes_length"] = a.value.len().into();
                                    summary["aes_hex"] =
                                        a.value.bytes().all(|b| b.is_ascii_hexdigit()).into();
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(quick_xml::events::Event::Eof) | Err(_) => break,
                    _ => {}
                }
            }
            std::fs::write(
                path.join("evidence.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .map_err(|_| AppError::internal())?;
            return Err(error);
        }
    };
    let before = app.receive_cursor_local(&account)?;
    let mut evidence = serde_json::json!({"started_at":chrono::Utc::now().to_rfc3339(),"abi":werc_wechat_win::native_messages::NATIVE_PROTOCOL_VERSION,
        "host_sha256":a[2],"messages_sent":false,"normal_user_store_accessed":false,"native_download_attempted":false,
        "declared_size":descriptor.media.declared_size,"complete_release":false,"product_video_link_implemented":false});
    let save = |v: &serde_json::Value| {
        std::fs::write(
            path.join("evidence.json"),
            serde_json::to_vec_pretty(v).unwrap(),
        )
        .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    let client_path = client::executable(a[3].parse().map_err(|_| AppError::internal())?)?;
    provider.connect_pinned(
        &account,
        &PathBuf::from(&a[1]),
        &a[2],
        a[3].parse().map_err(|_| AppError::internal())?,
        &client_path,
    )?;
    provider.mark_ready(&account)?;
    let context = provider.resolve_send_context(&account, "filehelper")?;
    if row.payload.sender_id != context.sender_id {
        provider.disconnect(None)?;
        return Err(AppError::new(
            "IDENTITY_CHANGED",
            "test observation belongs to another login",
        ));
    }
    evidence["native_download_attempted"] = true.into();
    save(&evidence)?;
    let result = provider.download_received_video(&context, &descriptor);
    let same_context = provider
        .resolve_send_context(&account, "filehelper")
        .is_ok_and(|c| c == context);
    provider.disconnect(None)?;
    evidence["controller_closed"] = true.into();
    let bytes = match result {
        Ok(bytes) if same_context => bytes,
        Ok(_) => {
            evidence["error_code"] = "IDENTITY_CHANGED".into();
            save(&evidence)?;
            return Err(AppError::internal());
        }
        Err(e) => {
            evidence["error_code"] = e.code.clone().into();
            evidence["error_detail"] = e.message.clone().into();
            save(&evidence)?;
            return Err(e);
        }
    };
    evidence["downloaded_bytes"] = bytes.len().into();
    evidence["sha256"] = hex::encode(Sha256::digest(&bytes)).into();
    evidence["mp4_ftyp"] = (bytes.get(4..8) == Some(b"ftyp")).into();
    // Research capture is encrypted through existing file storage. It is not a
    // persistent type43 message association or a public video capability.
    let mut assets = Vec::new();
    for chunk in bytes.chunks(werc_domain::file::MAX_FILE_BYTES) {
        assets.push(app.upload_file_local(
            &account,
            "native-video-research-part.bin",
            chunk.to_vec(),
        )?);
    }
    std::fs::write(
        path.join("private-asset-ids.json"),
        serde_json::to_vec(&assets).unwrap(),
    )
    .map_err(|_| AppError::internal())?;
    evidence["encrypted_research_capture"] = true.into();
    evidence["encrypted_research_parts"] = assets.len().into();
    evidence["gap_cursor_unchanged"] = (before == app.receive_cursor_local(&account)?).into();
    evidence["outbox_empty"] = app.sends_local(&account)?.is_empty().into();
    save(&evidence)?;
    evidence["decode"] = match decode(Path::new(&a[6]), &bytes) {
        Ok(decoded) => decoded,
        Err(e) => {
            evidence["decode_error"] = e.code.clone().into();
            save(&evidence)?;
            return Err(e);
        }
    };
    drop(bytes);
    drop(rows);
    drop(app);
    drop(provider);
    drop(repo);
    let offline = Application::new(
        Arc::new(SqliteRepository::open(&db)?),
        Arc::new(WindowsWechatProvider),
    );
    let principal = Principal {
        id: "owned-video-research".into(),
        account_ids: vec![account.clone()],
        scopes: vec!["media:read".into()],
    };
    let mut restored = zeroize::Zeroizing::new(Vec::new());
    for asset in &assets {
        restored.extend_from_slice(
            &offline
                .download_file(&principal, &account, &asset.id)?
                .bytes,
        );
    }
    if hex::encode(Sha256::digest(&restored)) != evidence["sha256"].as_str().unwrap() {
        return Err(AppError::internal());
    }
    evidence["reopen_decode"] = decode(Path::new(&a[6]), &restored)?;
    evidence["encrypted_store_reopen_matched"] = true.into();
    evidence["finished_at"] = chrono::Utc::now().to_rfc3339().into();
    save(&evidence)?;
    println!("{evidence}");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1);
    }
}
