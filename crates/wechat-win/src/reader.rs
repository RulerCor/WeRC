use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::mpsc::{self, Receiver};
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use werc_domain::history::{HISTORY_BATCH_BYTES, HISTORY_BATCH_ROWS, HistoryReadEvent};
use werc_domain::{
    AppError, AppResult,
    history::{HistoryBundle, HistoryReadRequest},
};
use zeroize::Zeroizing;

const PROGRESS_TIMEOUT: Duration = Duration::from_secs(120);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
fn protocol_error() -> AppError {
    AppError::new("READER_PROTOCOL_INVALID", "数据执行器流协议不匹配")
}
fn receive<T>(receiver: &Receiver<T>, started: Instant) -> AppResult<T> {
    receive_bounded(receiver, started, PROGRESS_TIMEOUT, TOTAL_TIMEOUT)
}
fn receive_bounded<T>(
    receiver: &Receiver<T>,
    started: Instant,
    progress: Duration,
    total: Duration,
) -> AppResult<T> {
    let remaining = total
        .checked_sub(started.elapsed())
        .ok_or_else(|| AppError::new("READER_TIMEOUT", "只读数据处理超过总时限"))?;
    receiver
        .recv_timeout(progress.min(remaining))
        .map_err(|e| match e {
            mpsc::RecvTimeoutError::Timeout => {
                AppError::new("READER_TIMEOUT", "只读数据处理启动或进展超时")
            }
            _ => protocol_error(),
        })
}

pub fn execute_stream(
    executable: &Path,
    expected_sha256: &str,
    request: &HistoryReadRequest,
    visitor: &mut dyn FnMut(HistoryReadEvent) -> AppResult<()>,
) -> AppResult<()> {
    let _component = verify_worker(executable, expected_sha256)?;
    let input = Zeroizing::new(
        serde_json::to_vec(&Request {
            path: &request.path,
            key_hex: request.key_hex.as_deref(),
            allow_plaintext: request.allow_plaintext,
        })
        .map_err(|_| AppError::internal())?,
    );
    if input.len() > 65536 {
        return Err(AppError::new("INVALID_ARGUMENT", "数据来源请求过长"));
    }
    let mut command = Command::new(executable);
    command
        .arg("--stream-v3")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(executable.parent().ok_or_else(AppError::internal)?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| AppError::new("READER_UNAVAILABLE", "无法启动只读数据执行器"))?;
    let started = Instant::now();
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(protocol_error());
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(protocol_error());
    };
    // Reads and writes both live off-thread: even a worker that stops accepting
    // stdin is covered by the startup/progress and absolute deadline.
    let (writes, write_rx) = mpsc::sync_channel::<Zeroizing<Vec<u8>>>(1);
    let (written_tx, written) = mpsc::sync_channel(1);
    let writer = std::thread::spawn(move || {
        for bytes in write_rx {
            let result = stdin
                .write_all(&bytes)
                .and_then(|_| stdin.flush())
                .map_err(|_| ());
            let failed = result.is_err();
            if written_tx.send(result).is_err() || failed {
                break;
            }
        }
    });
    let (frames_tx, frames) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        loop {
            let result = (|| -> Result<Option<Zeroizing<Vec<u8>>>, ()> {
                let mut prefix = [0u8; 4];
                match stdout.read(&mut prefix[..1]) {
                    Ok(0) => return Ok(None),
                    Ok(1) => (),
                    _ => return Err(()),
                }
                stdout.read_exact(&mut prefix[1..]).map_err(|_| ())?;
                let length = u32::from_le_bytes(prefix) as usize;
                if length == 0 || length > HISTORY_BATCH_BYTES + 65536 {
                    return Err(());
                }
                let mut bytes = Zeroizing::new(vec![0u8; length]);
                stdout.read_exact(&mut bytes).map_err(|_| ())?;
                Ok(Some(bytes))
            })();
            let stop = !matches!(result, Ok(Some(_)));
            if frames_tx.send(result).is_err() || stop {
                break;
            }
        }
    });
    let result = (|| -> AppResult<()> {
        let mut initial = Zeroizing::new(Vec::with_capacity(input.len() + 4));
        initial.extend_from_slice(&(input.len() as u32).to_le_bytes());
        initial.extend_from_slice(&input);
        drop(input);
        writes.send(initial).map_err(|_| protocol_error())?;
        receive(&written, started)?.map_err(|_| protocol_error())?;
        let mut validator = StreamValidator::default();
        loop {
            let bytes = receive(&frames, started)?
                .map_err(|_| protocol_error())?
                .ok_or_else(protocol_error)?;
            validate_wire(&bytes)?;
            let event: HistoryReadEvent =
                serde_json::from_slice(&bytes).map_err(|_| protocol_error())?;
            let ack = validator.accept(&event)?;
            drop(bytes);
            if let HistoryReadEvent::Error { code } = &event {
                return Err(AppError::new(code, "来源未通过只读流检查；未修改来源"));
            }
            let ended = matches!(event, HistoryReadEvent::End { .. });
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| visitor(event))).map_err(
                |_| AppError::new("READER_CALLBACK_FAILED", "流读取回调异常；已中止自有执行器"),
            )??;
            // A synchronous user callback cannot be preempted, but no ACK or new
            // data is accepted if it returns after the absolute deadline.
            if started.elapsed() >= TOTAL_TIMEOUT {
                return Err(AppError::new("READER_TIMEOUT", "只读数据处理超过总时限"));
            }
            writes
                .send(Zeroizing::new(ack.to_le_bytes().to_vec()))
                .map_err(|_| protocol_error())?;
            receive(&written, started)?.map_err(|_| protocol_error())?;
            if ended {
                // Includes partial trailing frames: EOF is the ONLY legal item.
                if receive(&frames, started)?
                    .map_err(|_| protocol_error())?
                    .is_some()
                {
                    return Err(protocol_error());
                }
                let exiting = Instant::now();
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) if status.success() => return Ok(()),
                        Ok(Some(_)) | Err(_) => return Err(protocol_error()),
                        Ok(None)
                            if started.elapsed() < TOTAL_TIMEOUT
                                && exiting.elapsed() < PROGRESS_TIMEOUT =>
                        {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        _ => return Err(AppError::new("READER_TIMEOUT", "执行器结束超时")),
                    }
                }
            }
        }
    })();
    // Only this Command's child is terminated, never process-name / PID scans.
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    drop(writes);
    drop(written);
    drop(frames);
    let writer_joined = writer.join();
    let reader_joined = reader.join();
    if writer_joined.is_err() || reader_joined.is_err() {
        return Err(protocol_error());
    }
    result
}

fn validate_wire(bytes: &[u8]) -> AppResult<()> {
    use serde_json::Value;
    fn keys(value: &Value, allowed: &[&str]) -> AppResult<()> {
        let object = value.as_object().ok_or_else(protocol_error)?;
        if object.len() != allowed.len() || allowed.iter().any(|key| !object.contains_key(*key)) {
            return Err(protocol_error());
        }
        if object
            .values()
            .any(|v| v.as_str().is_some_and(|s| s.len() > 1024 * 1024))
        {
            return Err(protocol_error());
        }
        Ok(())
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| protocol_error())?;
    if value.get("kind").and_then(Value::as_str) == Some("batch") {
        let data = &value["data"];
        keys(
            data,
            &[
                "schema",
                "source_sha256",
                "encryption",
                "wal_committed_frames",
                "messages",
                "contacts",
                "sessions",
                "warnings",
            ],
        )?;
        for (name, allowed) in [
            (
                "messages",
                &[
                    "id",
                    "conversation_id",
                    "sender_id",
                    "local_id",
                    "server_id",
                    "timestamp",
                    "sequence",
                    "message_type",
                    "text",
                    "content_status",
                ][..],
            ),
            ("contacts", &["username", "nickname", "remark", "alias"][..]),
            (
                "sessions",
                &["username", "summary", "timestamp", "unread_count"][..],
            ),
        ] {
            for item in data[name].as_array().ok_or_else(protocol_error)? {
                keys(item, allowed)?;
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct StreamValidator {
    metadata: Option<(String, String, String, usize)>,
    batches: u64,
    counts: [usize; 3],
    ended: bool,
}
impl StreamValidator {
    fn accept(&mut self, event: &HistoryReadEvent) -> AppResult<u64> {
        if self.ended {
            return Err(protocol_error());
        }
        match event {
            HistoryReadEvent::Begin {
                schema,
                source_sha256,
                encryption,
                wal_committed_frames,
            } if self.metadata.is_none() => {
                if schema != "wechat4_columns_v1"
                    || !valid_digest(source_sha256)
                    || !matches!(
                        encryption.as_str(),
                        "plaintext_explicit" | "sqlcipher4_rawkey_4096_sha512"
                    )
                    || *wal_committed_frames > 128 * 1024 * 1024 / 4120
                {
                    return Err(protocol_error());
                }
                self.metadata = Some((
                    schema.clone(),
                    source_sha256.clone(),
                    encryption.clone(),
                    *wal_committed_frames,
                ));
                Ok(u64::MAX)
            }
            HistoryReadEvent::Batch {
                sequence,
                digest,
                data,
            } => {
                let Some(meta) = &self.metadata else {
                    return Err(protocol_error());
                };
                let counts = [
                    data.messages.len(),
                    data.contacts.len(),
                    data.sessions.len(),
                ];
                let rows = counts.iter().sum::<usize>();
                if *sequence != self.batches
                    || !valid_digest(digest)
                    || rows == 0
                    || rows > HISTORY_BATCH_ROWS
                    || data.warnings.len() > 128
                    || data.warnings.iter().any(|v| v.len() > 256)
                    || meta
                        != &(
                            data.schema.clone(),
                            data.source_sha256.clone(),
                            data.encryption.clone(),
                            data.wal_committed_frames,
                        )
                {
                    return Err(protocol_error());
                }
                let encoded =
                    Zeroizing::new(serde_json::to_vec(data).map_err(|_| protocol_error())?);
                if encoded.len() > HISTORY_BATCH_BYTES
                    || hex::encode(Sha256::digest(encoded.as_slice())) != *digest
                {
                    return Err(protocol_error());
                }
                for (total, count) in self.counts.iter_mut().zip(counts) {
                    *total = total.checked_add(count).ok_or_else(protocol_error)?;
                }
                self.batches = self.batches.checked_add(1).ok_or_else(protocol_error)?;
                Ok(*sequence)
            }
            HistoryReadEvent::End {
                batches,
                message_count,
                contact_count,
                session_count,
            } if self.metadata.is_some() => {
                if *batches != self.batches
                    || [*message_count, *contact_count, *session_count] != self.counts
                {
                    return Err(protocol_error());
                }
                self.ended = true;
                Ok(u64::MAX - 1)
            }
            HistoryReadEvent::Error { code }
                if !code.is_empty()
                    && code.len() <= 128
                    && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') =>
            {
                self.ended = true;
                Ok(0)
            }
            _ => Err(protocol_error()),
        }
    }
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    fn begin() -> HistoryReadEvent {
        HistoryReadEvent::Begin {
            schema: "wechat4_columns_v1".into(),
            source_sha256: "0".repeat(64),
            encryption: "plaintext_explicit".into(),
            wal_committed_frames: 0,
        }
    }
    fn batch() -> HistoryReadEvent {
        let data = HistoryBundle {
            schema: "wechat4_columns_v1".into(),
            source_sha256: "0".repeat(64),
            encryption: "plaintext_explicit".into(),
            wal_committed_frames: 0,
            messages: vec![],
            contacts: vec![werc_domain::history::HistoryContact {
                username: "synthetic".into(),
                nickname: None,
                remark: None,
                alias: None,
            }],
            sessions: vec![],
            warnings: vec![],
        };
        let digest = hex::encode(Sha256::digest(serde_json::to_vec(&data).unwrap()));
        HistoryReadEvent::Batch {
            sequence: 0,
            digest,
            data,
        }
    }
    #[test]
    fn validates_sequence_digest_metadata_and_exact_end_counts() {
        let mut v = StreamValidator::default();
        assert!(v.accept(&batch()).is_err());
        assert_eq!(v.accept(&begin()).unwrap(), u64::MAX);
        assert!(v.accept(&begin()).is_err());
        let mut bad = batch();
        if let HistoryReadEvent::Batch { sequence, .. } = &mut bad {
            *sequence = 1;
        }
        assert!(v.accept(&bad).is_err());
        if let HistoryReadEvent::Batch {
            sequence, digest, ..
        } = &mut bad
        {
            *sequence = 0;
            *digest = "f".repeat(64);
        }
        assert!(v.accept(&bad).is_err());
        let mut bad = batch();
        if let HistoryReadEvent::Batch { data, .. } = &mut bad {
            data.source_sha256 = "1".repeat(64);
        }
        assert!(v.accept(&bad).is_err());
        assert_eq!(v.accept(&batch()).unwrap(), 0);
        assert!(v.accept(&batch()).is_err());
        assert!(
            v.accept(&HistoryReadEvent::End {
                batches: 1,
                message_count: 0,
                contact_count: 0,
                session_count: 0
            })
            .is_err()
        );
        assert_eq!(
            v.accept(&HistoryReadEvent::End {
                batches: 1,
                message_count: 0,
                contact_count: 1,
                session_count: 0
            })
            .unwrap(),
            u64::MAX - 1
        );
        assert!(v.accept(&begin()).is_err());
    }
    #[test]
    fn rejects_unknown_nested_fields_and_missing_nullable_fields() {
        let mut value = serde_json::to_value(batch()).unwrap();
        validate_wire(&serde_json::to_vec(&value).unwrap()).unwrap();
        value["data"]["contacts"][0]["unexpected"] = true.into();
        assert!(validate_wire(&serde_json::to_vec(&value).unwrap()).is_err());
        value["data"]["contacts"][0]
            .as_object_mut()
            .unwrap()
            .remove("unexpected");
        value["data"]["contacts"][0]
            .as_object_mut()
            .unwrap()
            .remove("alias");
        assert!(validate_wire(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn progress_and_absolute_deadlines_are_enforced() {
        let (_tx, rx) = mpsc::sync_channel::<()>(1);
        assert_eq!(
            receive_bounded(
                &rx,
                Instant::now(),
                Duration::from_millis(10),
                Duration::from_secs(1)
            )
            .unwrap_err()
            .code,
            "READER_TIMEOUT"
        );
        assert_eq!(
            receive_bounded(
                &rx,
                Instant::now() - Duration::from_secs(2),
                Duration::from_secs(1),
                Duration::from_secs(1)
            )
            .unwrap_err()
            .code,
            "READER_TIMEOUT"
        );
    }
    #[test]
    fn parent_transport_integration_when_fixture_is_supplied() {
        let Some(root) = std::env::var_os("WERC_STREAM_TEST_FIXTURES") else {
            return;
        };
        let executable = std::env::var_os("WERC_READER_EXE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(&root).join("worker.exe"));
        let hash = hex::encode(Sha256::digest(std::fs::read(&executable).unwrap()));
        let request = HistoryReadRequest {
            path: Path::new(&root)
                .join("large-stream.sqlite3")
                .to_string_lossy()
                .into(),
            key_hex: None,
            allow_plaintext: true,
        };
        let mut messages = 0;
        execute_stream(&executable, &hash, &request, &mut |event| {
            if let HistoryReadEvent::Batch { data, .. } = event {
                messages += data.messages.len();
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(messages, 60000);
        let error = execute_stream(&executable, &hash, &request, &mut |event| {
            if matches!(event, HistoryReadEvent::Batch { .. }) {
                Err(AppError::new(
                    "CALLBACK_REJECTED",
                    "synthetic callback failure",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.code, "CALLBACK_REJECTED");
        // Successful reopening proves the failed visitor killed/waited for its
        // worker and released both source and component locks.
        execute_stream(&executable, &hash, &request, &mut |_| Ok(())).unwrap();
    }
}

#[derive(Serialize)]
struct Request<'a> {
    path: &'a str,
    key_hex: Option<&'a str>,
    allow_plaintext: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    ok: bool,
    data: Option<HistoryBundle>,
    error: Option<String>,
}

pub fn execute(
    executable: &Path,
    expected_sha256: &str,
    request: &HistoryReadRequest,
) -> AppResult<HistoryBundle> {
    let _component = verify_worker(executable, expected_sha256)?;
    let input = Zeroizing::new(
        serde_json::to_vec(&Request {
            path: &request.path,
            key_hex: request.key_hex.as_deref(),
            allow_plaintext: request.allow_plaintext,
        })
        .map_err(|_| AppError::internal())?,
    );
    if input.len() > 65536 {
        return Err(AppError::new("INVALID_ARGUMENT", "数据来源请求过长"));
    }
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(executable.parent().ok_or_else(AppError::internal)?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| AppError::new("READER_UNAVAILABLE", "无法启动只读数据执行器"))?;
    let mut stdin = child.stdin.take().ok_or_else(AppError::internal)?;
    if stdin.write_all(&input).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(AppError::new("READER_FAILED", "执行器未接受本机请求"));
    }
    drop(stdin);
    let stdout = child.stdout.take().ok_or_else(AppError::internal)?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Zeroizing::new(Vec::new());
        stdout
            .take(32 * 1024 * 1024 + 65537)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        Ok::<_, ()>(bytes)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) if started.elapsed() < Duration::from_secs(45) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(AppError::new("READER_TIMEOUT", "只读数据处理超时或中断"));
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| AppError::internal())?
        .map_err(|_| AppError::new("READER_FAILED", "无法读取执行器响应"))?;
    if !status?.success() || bytes.len() > 32 * 1024 * 1024 + 65536 {
        return Err(AppError::new("READER_FAILED", "执行器异常退出或结果超限"));
    }
    let response: Response = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new("READER_PROTOCOL_INVALID", "数据执行器协议不匹配"))?;
    if response.ok {
        return response.data.ok_or_else(AppError::internal);
    }
    let code = response.error.unwrap_or_else(|| "READER_FAILED".into());
    let message = match code.as_str() {
        "SOURCE_BUSY_OR_DENIED" => {
            "来源仍有写入者或无读取权限；请提供关闭后的离线副本，不要只复制主DB而遗漏WAL"
        }
        "KEY_OR_PAGE_AUTHENTICATION_FAILED" | "KEY_FORMAT_INVALID" | "KEY_REQUIRED" => {
            "密钥未提供、格式错误或页面认证失败；未保存读取结果"
        }
        "SCHEMA_UNSUPPORTED" => "当前数据库表结构未适配；没有把空结果当作成功",
        _ => "来源未通过只读数据检查；未修改来源，详情见错误代码",
    };
    Err(AppError::new(&code, message))
}

fn verify_worker(executable: &Path, expected_sha256: &str) -> AppResult<std::fs::File> {
    if expected_sha256.len() != 64 {
        return Err(AppError::new(
            "READER_UNAVAILABLE",
            "此构建未绑定数据执行器摘要",
        ));
    }
    let metadata = std::fs::symlink_metadata(executable)
        .map_err(|_| AppError::new("READER_UNAVAILABLE", "缺少配套只读数据执行器"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 64 * 1024 * 1024
    {
        return Err(AppError::new(
            "COMPONENT_INTEGRITY_FAILED",
            "数据执行器文件不符合此构建清单",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(AppError::new(
                "COMPONENT_INTEGRITY_FAILED",
                "数据执行器不能是重解析路径",
            ));
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    // 校验句柄保持打开至子进程结束，禁止在校验和装载之间替换配套文件。
    let mut component = options
        .open(executable)
        .map_err(|_| AppError::new("READER_UNAVAILABLE", "无法锁定配套执行器"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = component
            .read(&mut buffer)
            .map_err(|_| AppError::internal())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if hex::encode(hash.finalize()) != expected_sha256 {
        return Err(AppError::new(
            "COMPONENT_INTEGRITY_FAILED",
            "数据执行器与此构建摘要不匹配",
        ));
    }
    Ok(component)
}
