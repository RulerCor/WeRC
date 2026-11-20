//! Offline, redacted format analysis of the isolated encrypted image capture.
use std::{path::PathBuf, sync::Arc};
use werc_application::Application;
use werc_domain::{AppError, AppResult, Principal};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;
fn run() -> AppResult<()> {
    let a: Vec<_> = std::env::args().collect();
    if a.len() != 2 {
        return Err(AppError::internal());
    }
    let app = Application::new(
        Arc::new(SqliteRepository::open(&PathBuf::from(&a[1]))?),
        Arc::new(WindowsWechatProvider),
    );
    let accounts = app.accounts_local()?;
    if accounts.len() != 1 {
        return Err(AppError::internal());
    }
    let account = &accounts[0].id;
    let p = Principal {
        id: "owned-inspection".into(),
        account_ids: vec![account.clone()],
        scopes: vec!["media:read".into()],
    };
    for asset in app.file_assets_local(account)? {
        let content = app.download_file(&p, account, &asset.id)?;
        if content.file_name != "received-native-research.bin" {
            continue;
        }
        let bytes = &content.bytes;
        let at = bytes
            .windows(2)
            .rposition(|w| w == [255, 217])
            .map(|n| n + 2)
            .ok_or_else(AppError::internal)?;
        let tail = &bytes[at..];
        let decoded = werc_domain::image::canonical_received_png(&bytes[..at]);
        let result = serde_json::json!({"bytes":bytes.len(),"trailing_bytes":tail.len(),"tail_all_zero":tail.iter().all(|b|*b==0),"pkcs7":!tail.is_empty() && tail.len()<=16 && tail.iter().all(|b|*b==tail.len() as u8),"trimmed_decode":match decoded {Ok((info,png))=>serde_json::json!({"width":info.width,"height":info.height,"png_bytes":png.len()}),Err(e)=>serde_json::json!({"error_code":e.code})}});
        println!("{result}");
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", e.code);
        std::process::exit(1);
    }
}
