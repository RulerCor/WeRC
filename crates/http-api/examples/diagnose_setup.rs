use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rusqlite::{Connection, OpenFlags};
use std::{io::Read, path::Path};
use werc_application::ports::CapabilityProvider;
use werc_domain::{
    history::{HistoryReadEvent, HistoryReadRequest},
    onboarding::{ProtectedSetup, SetupRun},
};
use werc_wechat_win::{WindowsWechatProvider, discovery, key_acquisition, reader};
use zeroize::Zeroizing;

// 本机显式只读诊断：不推进引导，不改产品状态，不输出密钥、身份或正文。
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2
        || !matches!(
            args[0].as_str(),
            "--status" | "--probe" | "--capture" | "--capture-fts"
        )
    {
        return Err(
            "usage: --status <state.sqlite3> | --probe <state.sqlite3> <worker> <sha256> <PID>"
                .into(),
        );
    }
    let connection = Connection::open_with_flags(&args[1], OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let payload: Vec<u8> = connection.query_row(
        "SELECT payload FROM setup_state WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let envelope: ProtectedSetup = serde_json::from_slice(&payload)?;
    let context = "WeRC/onboarding/v1";
    let provider = WindowsWechatProvider;
    let key = Zeroizing::new(
        provider
            .unprotect_key(context, &envelope.wrapped_key)
            .map_err(|e| e.code)?,
    );
    if envelope.nonce.len() != 12 {
        return Err("SETUP_CORRUPTED".into());
    }
    let plain = Zeroizing::new(
        Aes256Gcm::new_from_slice(&key)
            .map_err(|_| "KEY_INVALID")?
            .decrypt(
                Nonce::from_slice(&envelope.nonce),
                Payload {
                    msg: &envelope.ciphertext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| "SETUP_CORRUPTED")?,
    );
    let run: SetupRun = serde_json::from_slice(&plain)?;
    let parent = run
        .items
        .first()
        .and_then(|i| {
            Path::new(&i.path).ancestors().find(|p| {
                p.file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("db_storage"))
            })
        })
        .and_then(Path::parent)
        .and_then(Path::parent);
    println!(
        "{}",
        serde_json::json!({"phase":run.phase,"items":run.items.len(),"source_parent":parent,"client_version":run.binding.installation.version,"saved_pid":run.process_id,"read_only_state":true})
    );
    for (index, item) in run.items.iter().enumerate() {
        let name = Path::new(&item.path)
            .components()
            .rev()
            .take(2)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("/");
        println!(
            "{}",
            serde_json::json!({"index":index+1,"database":name,"key_ready":item.key_ready,"skipped":item.skipped,"error":item.error,"outcome":item.outcome,"imported":item.source.is_some()})
        );
    }
    if args[0] == "--status" {
        return Ok(());
    }
    if args.len() != 5 {
        return Err("probe requires state, worker, expected digest and explicit PID".into());
    }
    let pid: u32 = args[4].parse()?;
    let installations = discovery::discover().map_err(|e| e.code)?.installations;
    println!(
        "{}",
        serde_json::json!({"stage":"client_check","fingerprint_matches":installations.iter().any(|i|i.sha256==run.binding.installation.sha256),"pid_matches":installations.iter().any(|i|i.process_ids.contains(&pid))})
    );
    let installation = installations
        .iter()
        .find(|i| i.sha256 == run.binding.installation.sha256 && i.process_ids.contains(&pid))
        .ok_or("CLIENT_SELECTION_REQUIRED")?;
    let protected: Vec<u8> = connection.query_row(
        "SELECT protected FROM protected_keys WHERE account_id=?1",
        [&run.binding.account_id],
        |r| r.get(0),
    )?;
    let ring = Zeroizing::new(
        provider
            .unprotect_key(
                &format!(
                    "WeRC/key/v1/{}/{}",
                    run.binding.account_id, installation.sha256
                ),
                &protected,
            )
            .map_err(|e| e.code)?,
    );
    if !ring.starts_with(b"WRCKEY01") || !(ring.len() - 8).is_multiple_of(80) {
        return Err("KEY_PROTECTION_FAILED".into());
    }
    for (index, item) in run.items.iter().enumerate() {
        if args[0] == "--capture-fts" && !item.path.ends_with("_fts.db") {
            continue;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3);
        }
        let mut file = options.open(&item.path)?;
        let mut page = Zeroizing::new(vec![0u8; 4096]);
        file.read_exact(&mut page)?;
        let record = ring[8..]
            .as_chunks::<80>()
            .0
            .iter()
            .find(|r| r[..16] == page[..16])
            .or_else(|| {
                ring[8..]
                    .as_chunks::<80>()
                    .0
                    .iter()
                    .find(|r| werc_db_crypto::verify_first_page(&r[16..48], &page).is_ok())
            });
        let mut acquired = None;
        if record.is_none() {
            match key_acquisition::acquire(installation, pid, &item.path) {
                Ok(value) => {
                    println!(
                        "{}",
                        serde_json::json!({"index":index+1,"key_probe":"verified_from_process"})
                    );
                    acquired = Some(value);
                }
                Err(e) => println!(
                    "{}",
                    serde_json::json!({"index":index+1,"key_probe":e.code,"plaintext_header":page.starts_with(b"SQLite format 3\0"),"database_bytes":file.metadata()?.len()})
                ),
            }
        }
        let raw = record
            .map(|r| &r[16..48])
            .or_else(|| acquired.as_ref().map(|a| a.key.as_ref() as &[u8]));
        let Some(raw) = raw else {
            continue;
        };
        let capture = if args[0].starts_with("--capture") {
            Some(Capture::new(Path::new(&item.path))?)
        } else {
            None
        };
        let request = HistoryReadRequest {
            path: capture
                .as_ref()
                .map(|c| c.directory.join("source.db").to_string_lossy().into_owned())
                .unwrap_or_else(|| item.path.clone()),
            key_hex: Some(hex::encode(raw)),
            allow_plaintext: false,
        };
        let mut batches = 0usize;
        let mut messages = 0usize;
        let mut contacts = 0usize;
        let mut sessions = 0usize;
        let result =
            reader::execute_stream(Path::new(&args[2]), &args[3], &request, &mut |event| {
                if let HistoryReadEvent::Batch { data, .. } = event {
                    batches += 1;
                    messages += data.messages.len();
                    contacts += data.contacts.len();
                    sessions += data.sessions.len();
                }
                Ok(())
            });
        println!(
            "{}",
            serde_json::json!({"index":index+1,"reader":result.err().map(|e|e.code).unwrap_or("ok".into()),"batches":batches,"messages":messages,"contacts":contacts,"sessions":sessions,"stored_or_exported_content":false})
        );
    }
    Ok(())
}
fn shared(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3);
    }
    options.open(path)
}
// A stable observed byte capture is NOT an atomic/live SQLite snapshot guarantee.
// Both encrypted DB and WAL are copied; never create a main-DB-only live backup.
struct Capture {
    directory: std::path::PathBuf,
}
impl Capture {
    fn new(source: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        use sha2::{Digest, Sha256};
        let directory =
            std::env::temp_dir().join(format!("werc-private-capture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory)?;
        let capture = Self { directory };
        let wal = source.with_file_name(format!(
            "{}-wal",
            source.file_name().ok_or("INVALID_PATH")?.to_string_lossy()
        ));
        let mut digests = Vec::new();
        for (path, name) in [(source, "source.db"), (wal.as_path(), "source.db-wal")] {
            if !path.exists() {
                continue;
            }
            let mut input = shared(path)?;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(capture.directory.join(name))?;
            if input.metadata()?.len() > 256 * 1024 * 1024 {
                return Err("CAPTURE_LIMIT".into());
            }
            std::io::copy(&mut input, &mut output)?;
            drop(output);
            let mut digest = Sha256::new();
            let mut copy = std::fs::File::open(capture.directory.join(name))?;
            std::io::copy(&mut copy, &mut digest)?;
            digests.push((path.to_path_buf(), digest.finalize()));
        }
        for (path, expected) in digests {
            let mut digest = Sha256::new();
            std::io::copy(&mut shared(&path)?, &mut digest)?;
            if digest.finalize() != expected {
                return Err("SOURCE_CHANGED_DURING_CAPTURE".into());
            }
        }
        Ok(capture)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn main() {
    if run().is_err() {
        eprintln!("DIAGNOSTIC_FAILED: no secret or underlying error printed");
        std::process::exit(1);
    }
}
