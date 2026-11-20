//! Portable password-protected backup of local library documents and capture pages.
//! Each frame and the final count are authenticated; import is one transaction.
use crate::Application;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read, Write};
use werc_domain::{AppError, AppResult, library::MAX_DOCUMENT_BYTES};
use zeroize::Zeroizing;
const ROUNDS: u32 = 600_000;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format: String,
    version: u32,
    salt: String,
    nonce_prefix: String,
    rounds: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case", deny_unknown_fields)]
enum Frame {
    Document {
        id: String,
        kind: String,
        data: Value,
    },
    End {
        documents: u64,
    },
}
fn invalid() -> AppError {
    AppError::new(
        "BACKUP_INVALID",
        "备份密码错误、内容不完整或格式无效；未导入任何资料",
    )
}
fn key(password: &str, salt: &[u8]) -> AppResult<Zeroizing<[u8; 32]>> {
    if !(8..=1024).contains(&password.len()) {
        return Err(AppError::new("INVALID_ARGUMENT", "备份密码需为8至1024字节"));
    }
    let mut key = Zeroizing::new([0u8; 32]);
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), salt, ROUNDS, key.as_mut());
    Ok(key)
}
fn nonce(prefix: &[u8], sequence: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..4].copy_from_slice(prefix);
    n[4..].copy_from_slice(&sequence.to_be_bytes());
    n
}
fn line(reader: &mut dyn BufRead) -> AppResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_DOCUMENT_BYTES * 2 + 16384) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| invalid())?;
    if bytes.last() != Some(&b'\n') {
        return Err(invalid());
    }
    bytes.pop();
    Ok(bytes)
}
impl Application {
    pub fn backup_library_local(
        &self,
        account: &str,
        password: &str,
        request_id: &str,
        out: &mut dyn Write,
    ) -> AppResult<u64> {
        self.ensure_account(account)?;
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let mut salt = [0u8; 16];
        let mut prefix = [0u8; 4];
        getrandom::fill(&mut salt).map_err(|_| AppError::internal())?;
        getrandom::fill(&mut prefix).map_err(|_| AppError::internal())?;
        let key = key(password, &salt)?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| invalid())?;
        let header = Header {
            format: "WeRC-library-backup".into(),
            version: 1,
            salt: hex::encode(salt),
            nonce_prefix: hex::encode(prefix),
            rounds: ROUNDS,
        };
        let aad = serde_json::to_vec(&header).map_err(|_| invalid())?;
        out.write_all(&aad)
            .and_then(|_| out.write_all(b"\n"))
            .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "备份写入失败"))?;
        let write = |frame: Frame, sequence: u64, out: &mut dyn Write| -> AppResult<()> {
            let plain = Zeroizing::new(serde_json::to_vec(&frame).map_err(|_| invalid())?);
            let bytes = cipher
                .encrypt(
                    Nonce::from_slice(&nonce(&prefix, sequence)),
                    Payload {
                        msg: &plain,
                        aad: &aad,
                    },
                )
                .map_err(|_| invalid())?;
            writeln!(out, "{}", hex::encode(bytes))
                .map_err(|_| AppError::new("EXPORT_WRITE_FAILED", "备份写入失败"))
        };
        // Freeze the exported header set, then reject concurrent edits or captures.
        let mut before = None;
        let mut headers = Vec::new();
        loop {
            lease.check()?;
            let page = self.repository.library_list(account, "*", before, 100)?;
            headers.extend(page.data.into_iter().filter(|h| {
                !matches!(
                    h.kind.as_str(),
                    "export_plan"
                        | "image_cache_keys"
                        | "export_run"
                        | "article_plan"
                        | "article_run"
                        | "capture_media_job"
                        | "capture_asr_job"
                        | "send_exact"
                        | "moment_preservation"
                        | "message_preservation"
                        | "backfill_state"
                        | "backfill_page"
                )
            }));
            before = page.next_cursor;
            if before.is_none() {
                break;
            }
        }
        let mut count = 0;
        let included: std::collections::HashSet<_> =
            headers.iter().map(|h| h.id.as_str()).collect();
        for h in &headers {
            lease.check()?;
            let (current, mut data): (_, Value) = self.open_document(account, &h.id, &h.kind)?;
            if current.revision != h.revision {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "备份期间资料变化，请暂停补读或编辑后重试",
                ));
            }
            if h.kind == "web_article" { data["owner_run"]=Value::Null; }
            if h.kind == "capture_media" {
                let blob = data["blob_id"].as_str().ok_or_else(invalid)?;
                let count = data["chunks"].as_u64().ok_or_else(invalid)?;
                let owner = data["capture_id"].as_str().ok_or_else(invalid)?;
                if !included.contains(owner)
                    || count > headers.len() as u64
                    || (0..count).any(|i| !included.contains(format!("{blob}:{i}").as_str()))
                {
                    return Err(AppError::new(
                        "DOCUMENT_CONFLICT",
                        "媒体分块与归档不完整；未发布备份",
                    ));
                }
            }
            if h.kind == "moment_media" || h.kind == "article_image" {
                let blob = data["blob_id"].as_str().ok_or_else(invalid)?;
                let count = data["chunks"].as_u64().ok_or_else(invalid)?;
                if count == 0
                    || count > 16
                    || (0..count).any(|i| !included.contains(format!("{blob}:{i}").as_str()))
                {
                    return Err(invalid());
                }
            }
            if h.kind == "source_capture" {
                let pages = data["pages"].as_u64().ok_or_else(invalid)?;
                if pages > headers.len() as u64
                    || (0..pages).any(|i| !included.contains(format!("{}:{i}", h.id).as_str()))
                {
                    return Err(AppError::new(
                        "DOCUMENT_CONFLICT",
                        "补读进度与备份页不一致，请暂停补读后重试",
                    ));
                }
                data["imported"] = Value::Bool(true);
                if let Some(sources) = data["sources"].as_array_mut() {
                    for source in sources {
                        source["database"] = Value::String(String::new());
                    }
                }
            }
            write(
                Frame::Document {
                    id: h.id.clone(),
                    kind: h.kind.clone(),
                    data,
                },
                count,
                out,
            )?;
            count += 1;
        }
        // A changed job could otherwise reference pages outside this backup.
        for h in &headers {
            lease.check()?;
            if self.repository.library_get(account, &h.id)?.header.revision != h.revision {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "备份期间资料变化；未发布备份",
                ));
            }
        }
        write(Frame::End { documents: count }, count, out)?;
        Ok(count)
    }
    pub fn restore_library_local(
        &self,
        account: &str,
        password: &str,
        request_id: &str,
        input: &mut dyn BufRead,
    ) -> AppResult<u64> {
        self.ensure_account(account)?;
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let aad = line(input)?;
        let header: Header = serde_json::from_slice(&aad).map_err(|_| invalid())?;
        if header.format != "WeRC-library-backup" || header.version != 1 || header.rounds != ROUNDS
        {
            return Err(invalid());
        }
        let salt = hex::decode(header.salt).map_err(|_| invalid())?;
        let prefix = hex::decode(header.nonce_prefix).map_err(|_| invalid())?;
        if salt.len() != 16 || prefix.len() != 4 {
            return Err(invalid());
        }
        let key = key(password, &salt)?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| invalid())?;
        let namespace = uuid::Uuid::new_v4().to_string();
        let mut sequence = 0u64;
        let mut done = false;
        let rename = |id: &str| hex::encode(Sha256::digest(format!("{namespace}:{id}").as_bytes()));
        let mut captures = Vec::<(String, u64)>::new();
        let mut pages = std::collections::HashSet::<String>::new();
        let mut chunks = std::collections::HashSet::<String>::new();
        let mut required_chunks = std::collections::HashSet::<String>::new();
        let mut media = Vec::<(String, String, u64)>::new();
        let mut documents = std::iter::from_fn(|| {
            if done {
                return None;
            }
            let next = (|| {
                lease.check()?;
                let ciphertext = hex::decode(line(input)?).map_err(|_| invalid())?;
                let plain = Zeroizing::new(
                    cipher
                        .decrypt(
                            Nonce::from_slice(&nonce(&prefix, sequence)),
                            Payload {
                                msg: &ciphertext,
                                aad: &aad,
                            },
                        )
                        .map_err(|_| invalid())?,
                );
                let frame: Frame = serde_json::from_slice(&plain).map_err(|_| invalid())?;
                match frame {
                    Frame::End { documents } => {
                        if !required_chunks.is_subset(&chunks)
                            || documents != sequence
                            || !input.fill_buf().map_err(|_| invalid())?.is_empty()
                            || media.iter().any(|(owner, blob, count)| {
                                !captures.iter().any(|(id, _)| id == owner)
                                    || *count > chunks.len() as u64
                                    || (0..*count).any(|i| !chunks.contains(&format!("{blob}:{i}")))
                            })
                            || captures.iter().any(|(id, count)| {
                                *count > pages.len() as u64
                                    || (0..*count).any(|i| !pages.contains(&format!("{id}:{i}")))
                            })
                        {
                            return Err(invalid());
                        }
                        done = true;
                        Ok(None)
                    }
                    Frame::Document { id, kind, mut data } => {
                        if id.is_empty()
                            || id.len() > 128
                            || !data.is_object()
                            || !id
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':'))
                            || !matches!(
                                kind.as_str(),
                                "favorite"
                                    | "note"
                                    | "article"
                                    | "web_article"
                                    | "article_image"
                                    | "contact_avatar"
                                    | "template"
                                    | "transcript"
                                    | "report"
                                    | "moment_preserved"
                                    | "message_preserved"
                                    | "moment_media"
                                    | "source_capture"
                                    | "capture_page"
                                    | "capture_change"
                                    | "capture_media"
                                    | "media_chunk"
                            )
                        {
                            return Err(invalid());
                        }
                        let mut new_id = rename(&id);
                        if kind == "web_article" { data["owner_run"]=Value::Null; }
                        if kind == "transcript" {
                            if let Some(source) = data["source"].as_str() {
                                if let Ok(mut value) = serde_json::from_str::<Value>(source) {
                                    if value["kind"] == "archived" {
                                        value["capture_id"] = Value::String(rename(
                                            value["capture_id"].as_str().ok_or_else(invalid)?,
                                        ));
                                        data["source"] = Value::String(
                                            serde_json::to_string(&value).map_err(|_| invalid())?,
                                        );
                                    }
                                }
                            }
                        }
                        if kind == "source_capture" {
                            data["imported"] = Value::Bool(true);
                            captures.push((
                                new_id.clone(),
                                data["pages"].as_u64().ok_or_else(invalid)?,
                            ));
                        }
                        if kind == "capture_page" {
                            let owner = rename(data["capture_id"].as_str().ok_or_else(invalid)?);
                            new_id =
                                format!("{owner}:{}", data["index"].as_u64().ok_or_else(invalid)?);
                            data["capture_id"] = Value::String(owner);
                            pages.insert(new_id.clone());
                        }
                        if kind == "capture_media" {
                            let owner = rename(data["capture_id"].as_str().ok_or_else(invalid)?);
                            let blob = rename(data["blob_id"].as_str().ok_or_else(invalid)?);
                            let page = data["page"].as_u64().ok_or_else(invalid)?;
                            let row = data["row"].as_u64().ok_or_else(invalid)?;
                            new_id = format!("{owner}:media:{page}:{row}");
                            media.push((
                                owner.clone(),
                                blob.clone(),
                                data["chunks"].as_u64().ok_or_else(invalid)?,
                            ));
                            data["capture_id"] = Value::String(owner);
                            data["blob_id"] = Value::String(blob);
                        }
                        if kind == "moment_media" || kind == "article_image" {
                            let blob = rename(data["blob_id"].as_str().ok_or_else(invalid)?);
                            data["blob_id"] = Value::String(blob.clone());
                            data["id"] = Value::String(new_id.clone());
                            let count = data["chunks"]
                                .as_u64()
                                .filter(|n| *n > 0 && *n <= 16)
                                .ok_or_else(invalid)?;
                            for index in 0..count {
                                required_chunks.insert(format!("{blob}:{index}"));
                            }
                        }
                        if kind == "media_chunk" {
                            let blob = rename(data["blob_id"].as_str().ok_or_else(invalid)?);
                            new_id =
                                format!("{blob}:{}", data["index"].as_u64().ok_or_else(invalid)?);
                            data["blob_id"] = Value::String(blob);
                            chunks.insert(new_id.clone());
                        }
                        if kind == "capture_change" {
                            let owner = rename(data["capture_id"].as_str().ok_or_else(invalid)?);
                            data["capture_id"] = Value::String(owner);
                            data["state"] = Value::String("archived".into());
                        }
                        let document =
                            self.seal_document_unchecked(account, &new_id, &kind, 1, &data)?;
                        sequence = sequence.checked_add(1).ok_or_else(invalid)?;
                        Ok(Some(document))
                    }
                }
            })();
            match next {
                Ok(Some(d)) => Some(Ok(d)),
                Ok(None) => None,
                Err(e) => {
                    done = true;
                    Some(Err(e))
                }
            }
        });
        self.repository.library_import(account, &mut documents)
    }
}
