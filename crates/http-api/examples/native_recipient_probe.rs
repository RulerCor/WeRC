//! Explicit no-send constructor checks for the authenticated self endpoint.
#![cfg(windows)]
#[path = "support/client.rs"]
mod client;
use std::path::PathBuf;
use werc_application::ports::CapabilityProvider;
use werc_domain::{AppError, AppResult};
use werc_wechat_win::native_messages::{NATIVE_PROTOCOL_VERSION, NativeFileHelperProvider};

fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 8 || a[1] != "--construct-only-no-send" {
        return Err(AppError::new(
            "ARGUMENTS",
            "mode host host_hash pid codec codec_hash new_evidence_dir",
        ));
    }
    let directory = PathBuf::from(&a[7]);
    std::fs::create_dir(&directory)
        .map_err(|_| AppError::new("STATE_EXISTS", "使用全新验证目录"))?;
    let pid = a[4].parse().map_err(|_| AppError::internal())?;
    let path = client::executable(pid)?;
    let p = NativeFileHelperProvider::connect(
        "isolated-recipient-probe",
        &PathBuf::from(&a[2]),
        &a[3],
        pid,
        &path,
    )?
    .with_voice_codec(PathBuf::from(&a[5]), a[6].clone());
    let baseline = p.resolve_send_context("isolated-recipient-probe", "filehelper")?;
    let context = p.resolve_send_context("isolated-recipient-probe", &baseline.sender_id)?;
    let mut evidence = serde_json::json!({"passed":false,"abi":NATIVE_PROTOCOL_VERSION,"started_at":chrono::Utc::now().to_rfc3339(),"host_sha256":a[3],"codec_sha256":a[6],"target_is_authenticated_self":context.sender_id==context.target_id,"target_longer_than_inline_string":context.target_id.len()>15,"native_voice_constructed":false,"native_image_constructed":false,"native_file_constructed":false,"native_video_constructed":false,"native_emoticon_constructed":false,"messages_sent":false,"normal_user_store_accessed":false,"complete_product":false});
    let save = |e: &serde_json::Value| {
        std::fs::write(
            directory.join("evidence.json"),
            serde_json::to_vec_pretty(e).unwrap(),
        )
        .map_err(|_| AppError::internal())
    };
    save(&evidence)?;
    // Owned tone bytes never leave the constructor/probe path.
    let mut wav = Vec::with_capacity(32044);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&32036u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16000u32.to_le_bytes());
    wav.extend_from_slice(&32000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&32000u32.to_le_bytes());
    for i in 0..16000 {
        let value = ((i as f64 * 440.0 * std::f64::consts::TAU / 16000.0).sin() * 6000.0) as i16;
        wav.extend_from_slice(&value.to_le_bytes());
    }
    let voice = p.encode_voice(&wav)?;
    evidence["native_voice_constructed"] = p.probe_voice_to(&context.target_id, &voice)?.into();
    save(&evidence)?;
    evidence["native_image_constructed"] = p
        .probe_image_to(
            &context.target_id,
            include_bytes!("../../../tests/fixtures/owned-image.png"),
        )?
        .into();
    save(&evidence)?;
    evidence["native_file_constructed"] = p
        .probe_file_to(
            &context.target_id,
            "WeRC-自动化测试.txt",
            "自动化测试：收件人构造验证".as_bytes(),
        )?
        .into();
    save(&evidence)?;
    let (video, seconds) = p.probe_video_to(
        &context.target_id,
        "WeRC-自动化测试.mp4",
        include_bytes!("../../../tests/fixtures/owned-send-video.mp4"),
    )?;
    evidence["native_video_constructed"] = video.into();
    evidence["video_native_seconds"] = seconds.into();
    save(&evidence)?;
    evidence["native_emoticon_constructed"] = p
        .probe_emoticon_to(
            &context.target_id,
            "WeRC-自动化测试.gif",
            include_bytes!("../../../tests/fixtures/owned-send-emoticon.gif"),
        )?
        .into();
    let good = [
        "native_voice_constructed",
        "native_image_constructed",
        "native_file_constructed",
        "native_video_constructed",
        "native_emoticon_constructed",
        "target_is_authenticated_self",
        "target_longer_than_inline_string",
    ]
    .iter()
    .all(|key| evidence[key] == true);
    evidence["passed"] = good.into();
    evidence["finished_at"] = chrono::Utc::now().to_rfc3339().into();
    save(&evidence)?;
    drop(p);
    if !good {
        return Err(AppError::new(
            "NATIVE_RECIPIENT_PROBE_FAILED",
            "仅构造验证未通过；未发送",
        ));
    }
    println!("{}", evidence);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1);
    }
}
