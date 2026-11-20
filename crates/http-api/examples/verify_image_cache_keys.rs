//! Native image-cache keys read only, transient encrypted store; no message or post submission.
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    sync::Arc,
};
use werc_application::{Application, ports::Repository};
use werc_domain::{AppError, AppResult};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{data_sources, discovery, managed_messages::ManagedWindowsWechatProvider};
fn error(code: &str) -> AppError {
    AppError::new(code, "只读补读与归档验证未通过")
}
fn run(
    _output: &Path,
    pid: u32,
    worker: &Path,
    hash: &str,
    host: &Path,
    host_hash: &str,
) -> AppResult<serde_json::Value> {
    let installations = discovery::discover()?.installations;
    let clients: Vec<_> = installations
        .iter()
        .filter(|i| i.process_ids.contains(&pid))
        .collect();
    if clients.len() != 1 || clients[0].version != "4.1.15.13" {
        return Err(error("CLIENT_SELECTION_REQUIRED"));
    }
    let catalog = data_sources::discover()?;
    if catalog.sources.len() != 1 {
        return Err(error("SOURCE_SELECTION_REQUIRED"));
    }
    let source = &catalog.sources[0];
    let root = Path::new(&source.account_directory);
    let canonical = root
        .canonicalize()
        .map_err(|_| error("SOURCE_UNAVAILABLE"))?
        .to_string_lossy()
        .to_lowercase();
    let fingerprint = hex::encode(Sha256::digest(
        format!("WeRC/setup-source/v1\0{canonical}").as_bytes(),
    ));
    let repo = Arc::new(SqliteRepository::in_memory()?);
    let account = repo
        .setup_account_for_source("isolated public avatar verification", &fingerprint)?
        .id;
    let provider = Arc::new(
        ManagedWindowsWechatProvider::with_live_reader(worker.to_owned(), hash.into())
            .with_live_window_limit(120)?,
    );
    let app = Application::new(repo.clone(), provider.clone());
    app.bind_client_local(&account, &clients[0].executable_path)?;
    provider.connect_pinned(&account, host, host_hash, pid, &clients[0].executable_path)?;
    let result = (|| {
        provider.mark_ready(&account)?;
        // Only this ephemeral product store receives recovered material.
        // Candidate paths and decoded image content are never printed or exported.
        let mut pending=vec![(root.join("msg/attach"),0usize)];let mut candidates=Vec::new();let mut visited=0usize;
        while let Some((directory,depth))=pending.pop(){
            if depth>10{continue;}
            let Ok(entries)=std::fs::read_dir(directory)else{continue};
            for entry in entries.flatten(){visited+=1;if visited>150_000{return Err(error("SAMPLE_DISCOVERY_LIMIT"));}
                let path=entry.path();let Ok(meta)=std::fs::symlink_metadata(&path)else{continue};
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes()&0x400!=0{continue;}
                if meta.is_dir(){pending.push((path,depth+1));}
                else if meta.is_file()&&meta.len()>64&&meta.len()<=8*1024*1024&&path.extension().is_some_and(|s|s.eq_ignore_ascii_case("dat")){
                    candidates.push((meta.modified().ok(),path));
                }
            }
        }
        candidates.sort_by(|a,b|b.0.cmp(&a.0));
        let mut errors=Vec::new();let mut verified=None;
        for(_,path)in candidates.iter().take(40){
            data_sources::validate_local_source(path)?;
            let sample=std::fs::read(path).map_err(|_|error("SAMPLE_UNAVAILABLE"))?;
            if !sample.starts_with(b"\x07\x08V2\x08\x07"){continue;}
            let before=hex::encode(Sha256::digest(&sample));
            match app.verify_image_keys_local(&account,sample.clone(),None,None,true,"readonly-image-cache-key"){
                Ok(result)=>{
                    let reopened=app.decode_cached_image_local(&account,sample,"readonly-image-cache-reopen")?;
                    let status=app.image_key_status_local(&account)?;
                    let after=hex::encode(Sha256::digest(std::fs::read(path).map_err(|_|error("SAMPLE_UNAVAILABLE"))?));
                    if before!=after||reopened.sha256!=result.sha256||!status.has_aes{return Err(error("IMAGE_VERIFICATION_MISMATCH"));}
                    verified=Some(serde_json::json!({"sample_sha256":before,"png_sha256":result.sha256,"png_bytes":result.png_hex.len()/2,"has_aes":status.has_aes,"has_xor":status.has_xor,"sample_bytes_unchanged":true,"encrypted_store_reopen":true}));break;
                },
                Err(e)=>{errors.push(e.code);if errors.len()>=3{break;}}
            }
        }
        if let Some(evidence)=verified{
            app.remove_image_keys_local(&account)?;
            Ok(serde_json::json!({"passed":true,"image":evidence,"previous_attempt_errors":errors,"messages_sent":false,"normal_product_state_opened":false,"complete_product":false}))
        }else{Ok(serde_json::json!({"passed":false,"error":"NO_VERIFIED_IMAGE_SAMPLE","attempt_errors":errors,"candidate_count":candidates.len(),"messages_sent":false,"normal_product_state_opened":false}))}
    })();
    provider.disconnect(None)?;
    app.clear_live_filehelper_local()?;
    result
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 7 || args[0] != "--consent-read-only-image-cache" {
        std::process::exit(2);
    }
    let output = Path::new(&args[6]);
    assert!(output.is_absolute() && !output.exists());
    std::fs::create_dir(output).unwrap();
    let value = match run(
        output,
        args[1].parse().expect("PID"),
        Path::new(&args[2]),
        &args[3],
        Path::new(&args[4]),
        &args[5],
    ) {
        Ok(v) => v,
        Err(e) => {
            serde_json::json!({"passed":false,"error":e.code,"detail":e.message,"messages_sent":false})
        }
    };
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("evidence.json"))
        .unwrap();
    f.write_all(&serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
    f.sync_all().unwrap();
    println!("{value}");
    if value["passed"] != true {
        std::process::exit(1);
    }
}
