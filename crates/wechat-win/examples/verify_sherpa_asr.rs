//! Local owned/public sample recognition. No WeChat, database or network access.
use std::{path::PathBuf,sync::atomic::AtomicBool};
use werc_wechat_win::{asr,sherpa_asr};
fn main(){
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    let run=||->Result<serde_json::Value,String>{
        if args.len()!=3{return Err("expected model-root model-id wav-file".into());}
        asr::configure_models(PathBuf::from(&args[0])).map_err(|e|e.code)?;
        let wav=std::fs::read(&args[2]).map_err(|e|e.to_string())?;
        let cancel=AtomicBool::new(false);let before=asr::cache_identity(&args[1],&cancel).map_err(|e|format!("{} {}",e.code,e.message))?;
        let transcript=asr::transcribe(&wav,&args[1],"auto",&cancel).map_err(|e|format!("{} {}",e.code,e.message))?;
        let after=asr::cache_identity(&args[1],&cancel).map_err(|e|e.code)?;
        if transcript.text.trim().is_empty()||before!=after{return Err("empty result or model changed".into());}
        Ok(serde_json::json!({"passed":true,"model":args[1],"text":transcript.text,"segments":transcript.segments.len(),"cache_identity_stable":true,"model_files":sherpa_asr::identity_files(&args[1]).map_err(|e|e.code)?.len(),"available":asr::status().map_err(|e|e.code)?.available}))
    };
    match run(){Ok(value)=>println!("{value}"),Err(error)=>{eprintln!("{error}");std::process::exit(1);}}
}
