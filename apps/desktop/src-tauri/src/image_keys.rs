use super::*;
use werc_application::image_cache_keys::*;
fn read_sample(path:&str)->Result<Vec<u8>,AppError>{
    use std::io::Read;
    let path=std::path::Path::new(path);werc_wechat_win::data_sources::validate_local_source(path)?;
    let mut options=std::fs::OpenOptions::new();options.read(true);
    #[cfg(windows)]{use std::os::windows::fs::OpenOptionsExt;options.share_mode(1).custom_flags(0x00200000);}
    let mut file=options.open(path).map_err(|_|AppError::new("SOURCE_ACCESS_DENIED","无法只读图片样本"))?;
    let before=file.metadata().map_err(|_|AppError::internal())?;
    if !before.is_file()||before.len()==0||before.len()>8*1024*1024{return Err(AppError::new("IMAGE_SAMPLE_INVALID","样本需为不超过8MiB的普通文件"));}
    let mut bytes=Vec::new();std::io::Read::by_ref(&mut file).take(8*1024*1024+1).read_to_end(&mut bytes).map_err(|_|AppError::new("SOURCE_READ_FAILED","图片样本读取失败"))?;
    let after=file.metadata().map_err(|_|AppError::internal())?;
    if bytes.len()as u64!=before.len()||before.len()!=after.len()||before.modified().ok()!=after.modified().ok(){return Err(AppError::new("SOURCE_CHANGED","读取期间图片样本已变化"));}Ok(bytes)
}
#[tauri::command]
pub(super) async fn image_key_status(state:State<'_,DesktopState>,account_id:String)->Result<ImageKeyStatus,AppError>{run_local(state,move|app|app.image_key_status_local(&account_id)).await}
#[tauri::command]
pub(super) async fn reveal_image_keys(state:State<'_,DesktopState>,account_id:String)->Result<ImageKeyReveal,AppError>{run_local(state,move|app|app.reveal_image_keys_local(&account_id)).await}
#[tauri::command]
pub(super) async fn remove_image_keys(state:State<'_,DesktopState>,account_id:String)->Result<(),AppError>{run_local(state,move|app|app.remove_image_keys_local(&account_id)).await}
#[tauri::command]
pub(super) async fn verify_image_keys(state:State<'_,DesktopState>,account_id:String,path:String,aes_hex:Option<String>,xor:Option<u8>,automatic:bool,request_id:String)->Result<ImageSample,AppError>{
    let aes=zeroize::Zeroizing::new(aes_hex);
    run_local(state,move|app|app.verify_image_keys_local(&account_id,read_sample(&path)?,aes.as_ref().cloned(),xor,automatic,&request_id)).await
}
#[tauri::command]
pub(super) async fn decode_image_sample(state:State<'_,DesktopState>,account_id:String,path:String,request_id:String)->Result<ImageSample,AppError>{run_local(state,move|app|app.decode_cached_image_local(&account_id,read_sample(&path)?,&request_id)).await}
