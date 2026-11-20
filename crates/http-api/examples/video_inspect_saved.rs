//! Offline inspection of the explicitly named encrypted research capture only.
use std::{path::PathBuf, sync::Arc};
use werc_application::Application;
use werc_domain::{AppError, AppResult, Principal, file::FileAsset};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;
fn run() -> AppResult<()> {
    let root = PathBuf::from(std::env::args().nth(1).ok_or_else(AppError::internal)?);
    let assets: Vec<FileAsset> = serde_json::from_slice(
        &std::fs::read(root.join("private-asset-ids.json")).map_err(|_| AppError::internal())?,
    )
    .map_err(|_| AppError::internal())?;
    if assets.is_empty() || assets.len() > 2 {
        return Err(AppError::internal());
    }
    let account = assets[0].account_id.clone();
    let app = Application::new(
        Arc::new(SqliteRepository::open(&root.join("state.sqlite3"))?),
        Arc::new(WindowsWechatProvider),
    );
    let p = Principal {
        id: "owned-video-parser".into(),
        account_ids: vec![account.clone()],
        scopes: vec!["media:read".into()],
    };
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    for asset in assets {
        if asset.account_id != account {
            return Err(AppError::internal());
        }
        bytes.extend_from_slice(&app.download_file(&p, &account, &asset.id)?.bytes);
    }
    let info = werc_domain::video::inspect_mp4(&bytes)?;
    let report = serde_json::json!({"passed":true,"container_inspection":info,"full_decode":false,"native_access":false,"messages_sent":false});
    std::fs::write(
        root.join("container-inspection.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .map_err(|_| AppError::internal())?;
    println!("{report}");
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1)
    }
}
