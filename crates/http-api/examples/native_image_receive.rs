//! Explicit local receive-only check using the isolated encrypted test snapshot.
//! No send/approval calls and no decrypted media or keys written to disk/logs.
#![cfg(windows)]
#[path = "support/client.rs"]
mod client;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
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

fn decoded_dimensions(bytes: &[u8]) -> AppResult<serde_json::Value> {
    // The owned verifier decodes only the bounded bytes sent via stdin; it does
    // not accept a file/URL, persist media, or display image content.
    let script = r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Drawing; $s=New-Object IO.MemoryStream; [Console]::OpenStandardInput().CopyTo($s); $s.Position=0; $img=[Drawing.Image]::FromStream($s,$false,$true); if($img.Width -gt 4096 -or $img.Height -gt 4096 -or ([long]$img.Width*$img.Height) -gt 4194304){throw 'dimensions'}; $b=New-Object Drawing.Bitmap($img); $p=$b.GetPixel(0,0); @{width=$b.Width;height=$b.Height;decoded=$true}|ConvertTo-Json -Compress; $b.Dispose();$img.Dispose();$s.Dispose()"#;
    let mut child = Command::new(
        PathBuf::from(std::env::var_os("SystemRoot").ok_or_else(AppError::internal)?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .args(["-NoProfile", "-NonInteractive", "-Command", script])
    .creation_flags(0x08000000)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|_| AppError::internal())?;
    child
        .stdin
        .take()
        .ok_or_else(AppError::internal)?
        .write_all(bytes)
        .map_err(|_| AppError::internal())?;
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().map_err(|_| AppError::internal())? {
            if !status.success() {
                return Err(AppError::new(
                    "IMAGE_DECODE_FAILED",
                    "owned decoder rejected image",
                ));
            }
            break;
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::new(
                "IMAGE_DECODE_TIMEOUT",
                "owned decoder timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or_else(AppError::internal)?
        .take(1024)
        .read_to_string(&mut output)
        .map_err(|_| AppError::internal())?;
    serde_json::from_str(output.trim_start_matches('\u{feff}')).map_err(|_| AppError::internal())
}
fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 6 && !(a.len() == 7 && a[6] == "capture") {
        return Err(AppError::new(
            "ARGUMENTS",
            "host host_hash pid fresh_directory isolated_snapshot",
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
    let client_path = client::executable(a[3].parse().map_err(|_| AppError::internal())?)?;
    let provider = Arc::new(ManagedWindowsWechatProvider::default());
    provider.connect_pinned(
        &account,
        &PathBuf::from(&a[1]),
        &a[2],
        a[3].parse().map_err(|_| AppError::internal())?,
        &client_path,
    )?;
    provider.mark_ready(&account)?;
    let app = Application::new(repo.clone(), provider.clone());
    app.bind_client_local(&account, &client_path)?;
    let rows = app.received_texts_local(&account, 0, 100)?;
    let position = rows
        .iter()
        .find(|r| r.payload.message.message_type == 3)
        .ok_or_else(AppError::internal)?
        .position;
    let before = app.receive_cursor_local(&account)?;
    let mut evidence = serde_json::json!({"started_at":chrono::Utc::now().to_rfc3339(),"abi":werc_wechat_win::native_messages::NATIVE_PROTOCOL_VERSION,"host_sha256":a[2],"messages_sent":false,"normal_user_store_accessed":false,"native_download_attempted":false,"media_downloaded":false,"complete_release":false});
    let save = |v: &serde_json::Value| {
        std::fs::write(
            path.join("evidence.json"),
            serde_json::to_vec_pretty(v).unwrap(),
        )
        .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    evidence["native_download_attempted"] = true.into();
    save(&evidence)?;
    if a.len() == 7 {
        let row = rows
            .iter()
            .find(|r| r.position == position)
            .ok_or_else(AppError::internal)?;
        let descriptor = werc_domain::received_media::image_descriptor(&row.payload.message)?;
        let context = provider.resolve_send_context(&account, "filehelper")?;
        let bytes = provider.download_received_image(&context, &descriptor)?;
        let format = if bytes.starts_with(b"\xff\xd8\xff") {
            "JPEG"
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "PNG"
        } else if bytes.starts_with(b"BM") {
            "BMP"
        } else if bytes.starts_with(b"GIF8") {
            "GIF"
        } else if bytes.starts_with(b"wxgf") {
            "WXGF"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "WEBP"
        } else {
            "unrecognized"
        };
        evidence["raw_format"] = format.into();
        evidence["raw_bytes"] = bytes.len().into();
        evidence["raw_jpeg_eoi_at_end"] = bytes.ends_with(&[255, 217]).into();
        evidence["raw_decoder"] = match decoded_dimensions(&bytes) {
            Ok(v) => v,
            Err(e) => serde_json::json!({"error_code":e.code}),
        };
        let asset =
            app.upload_file_local(&account, "received-native-research.bin", bytes.to_vec())?;
        evidence["raw_encrypted_research_capture"] = true.into();
        std::fs::write(path.join("private-asset-id.txt"), asset.id)
            .map_err(|_| AppError::internal())?;
        drop(bytes);
        drop(rows);
        drop(app);
        provider.disconnect(None)?;
        drop(provider);
        drop(repo);
        evidence["controller_closed"] = true.into();
        evidence["finished_at"] = chrono::Utc::now().to_rfc3339().into();
        save(&evidence)?;
        println!("{evidence}");
        return Ok(());
    }
    let mut concurrent_checks = 0u64;
    let result = std::thread::scope(|scope| {
        let task = scope.spawn(|| app.receive_image_file_local(&account, position));
        while !task.is_finished() {
            provider.resolve_send_context(&account, "filehelper")?;
            concurrent_checks += 1;
            std::thread::sleep(Duration::from_millis(10));
        }
        task.join().map_err(|_| AppError::internal())?
    });
    evidence["concurrent_context_reads"] = concurrent_checks.into();
    let asset = match result {
        Ok(asset) => asset,
        Err(error) => {
            evidence["error_code"] = error.code.clone().into();
            evidence["error_detail"] = error.message.clone().into();
            drop(app);
            provider.disconnect(None)?;
            drop(provider);
            drop(repo);
            evidence["controller_closed"] = true.into();
            save(&evidence)?;
            return Err(error);
        }
    };
    let principal = Principal {
        id: "owned-local-receive-verification".into(),
        account_ids: vec![account.clone()],
        scopes: vec!["media:read".into()],
    };
    if app.receive_image_file_local(&account, position)?.id != asset.id {
        return Err(AppError::internal());
    }
    evidence["cached_asset_reused"] = true.into();
    let preview = app.received_image_preview_local(&account, position)?;
    let content = app.download_file(&principal, &account, &asset.id)?;
    if *preview != *content.bytes {
        return Err(AppError::internal());
    }
    drop(preview);
    let checksum = hex::encode(Sha256::digest(&content.bytes));
    evidence["decoded_image"] = decoded_dimensions(&content.bytes)?;
    evidence["media_downloaded"] = true.into();
    evidence["bytes"] = content.bytes.len().into();
    evidence["sha256"] = checksum.clone().into();
    evidence["gap_cursor_unchanged"] = (before == app.receive_cursor_local(&account)?).into();
    evidence["outbox_empty"] = app.sends_local(&account)?.is_empty().into();
    drop(content);
    drop(rows);
    drop(app);
    provider.disconnect(None)?;
    drop(provider);
    drop(repo);
    let repo = Arc::new(SqliteRepository::open(&db)?);
    let offline = Application::new(repo, Arc::new(WindowsWechatProvider));
    if offline.receive_image_file_local(&account, position)?.id != asset.id {
        return Err(AppError::internal());
    }
    let linked = offline.received_image_preview_local(&account, position)?;
    if hex::encode(Sha256::digest(&linked)) != checksum {
        return Err(AppError::internal());
    }
    evidence["v16_authenticated_link_reopened_offline"] = true.into();
    let reopened = offline.download_file(&principal, &account, &asset.id)?;
    if hex::encode(Sha256::digest(&reopened.bytes)) != checksum {
        return Err(AppError::internal());
    }
    evidence["reopened_image"] = decoded_dimensions(&reopened.bytes)?;
    evidence["encrypted_store_reopen_matched"] = true.into();
    evidence["controller_closed"] = true.into();
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
