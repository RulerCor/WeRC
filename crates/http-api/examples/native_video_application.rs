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
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult};
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
    let position = row.position;
    let before = app.receive_cursor_local(&account)?;
    let mut evidence = serde_json::json!({"started_at":chrono::Utc::now().to_rfc3339(),"abi":werc_wechat_win::native_messages::NATIVE_PROTOCOL_VERSION,"schema":19,"host_sha256":a[2],"messages_sent":false,"normal_user_store_accessed":false,"complete_release":false,"product_video_link_implemented":true});
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
    app.bind_client_local(&account, &client_path)?;
    let result = app.receive_video_local(&account, position);
    provider.disconnect(None)?;
    evidence["controller_closed"] = true.into();
    let asset = match result {
        Ok(v) => v,
        Err(e) => {
            evidence["error_code"] = e.code.clone().into();
            save(&evidence)?;
            return Err(e);
        }
    };
    let reused = app.receive_video_local(&account, position)?;
    if reused.id != asset.id {
        return Err(AppError::internal());
    }
    let bytes = app.received_video_content_local(&account, position)?;
    let sha = hex::encode(Sha256::digest(&bytes));
    if sha != asset.sha256
        || sha != "1a94b405748f15cb70a3ac5bc9597981b9fb001bd4fda0057692af45f636ea7d"
    {
        return Err(AppError::new(
            "CONTENT_MISMATCH",
            "previous isolated video digest differs",
        ));
    }
    let protected = repo.received_video(&account, &row.payload.id)?;
    if protected.ciphertext.len() != bytes.len() + 16
        || protected.ciphertext[..bytes.len()] == bytes[..]
    {
        return Err(AppError::internal());
    }
    evidence["sha256"] = sha.into();
    evidence["bytes"] = asset.size.into();
    evidence["video"] = serde_json::to_value(&asset.video).unwrap();
    evidence["cache_reused_after_disconnect"] = true.into();
    evidence["encrypted_video_row"] = true.into();
    evidence["gap_cursor_unchanged"] = (before == app.receive_cursor_local(&account)?).into();
    evidence["outbox_empty"] = app.sends_local(&account)?.is_empty().into();
    evidence["decode"] = decode(Path::new(&a[6]), &bytes)?;
    save(&evidence)?;
    drop(protected);
    drop(bytes);
    drop(rows);
    drop(app);
    drop(provider);
    drop(repo);
    let offline = Application::new(
        Arc::new(SqliteRepository::open(&db)?),
        Arc::new(WindowsWechatProvider),
    );
    let restored = offline.received_video_content_local(&account, position)?;
    if hex::encode(Sha256::digest(&restored)) != asset.sha256 {
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
