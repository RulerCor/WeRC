//! Text interchange adapters. ChatLab v0.0.2 and WeClone normalize_data headers.
use super::*;
use serde_json::{Value, json};
const COLUMNS: &[&str] = &[
    "sequence",
    "time",
    "conversation",
    "sender",
    "local_id",
    "server_id",
    "type",
    "text",
    "local_status",
    "nickname",
    "remark",
    "group_nickname",
    "attachment",
    "transcript",
];
pub fn capture_export_extension(format: &str) -> Option<&'static str> {
    Some(match format {
        "json" | "chatlab_json" => "json",
        "jsonl" | "chatlab_jsonl" => "jsonl",
        "csv" | "weclone_csv" => "csv",
        "chatlab_zip" => "zip",
        "xlsx" => "xlsx",
        "sql" => "sql",
        "txt" => "txt",
        "md" => "md",
        "html" => "html",
        _ => return None,
    })
}
pub(crate) fn validate_columns(columns: &[String]) -> AppResult<()> {
    if columns.len() > COLUMNS.len()
        || columns.iter().any(|s| !COLUMNS.contains(&s.as_str()))
        || columns.iter().collect::<HashSet<_>>().len() != columns.len()
    {
        return Err(AppError::new("INVALID_ARGUMENT", "Excel列无效或重复"));
    }
    Ok(())
}
fn message_type(value: &str) -> i32 {
    let full = value.parse::<u64>().unwrap_or(u64::MAX);
    match full & 0xffffffff {
        1 => 0,
        3 => 1,
        34 => 2,
        43 | 62 => 3,
        47 => 5,
        48 => 8,
        42 => 27,
        49 => match full >> 32 {
            6 => 4,
            5 => 7,
            19 => 26,
            3 | 4 | 33 | 36 => 24,
            57 => 25,
            2000 => 21,
            2001 => 20,
            _ => 99,
        },
        50 => 23,
        10000 => 80,
        10002 => 81,
        _ => 99,
    }
}
fn row_message_type(row: &CaptureRow) -> i32 {
    let raw = row.message.message_type.parse::<u64>().unwrap_or(u64::MAX);
    if raw & 0xffffffff == 49 {
        if let Some(subtype) = row.app_subtype {
            return message_type(&((u64::from(subtype) << 32) | 49).to_string());
        }
    }
    message_type(&row.message.message_type)
}
fn sql(value: &str) -> String {
    if value.contains('\0') {
        format!("CAST(X'{}' AS TEXT)", hex::encode(value))
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}
impl Application {
    fn capture_export_transcripts(
        &self,
        account: &str,
        id: &str,
        lease: &crate::history::QueryLease<'_>,
    ) -> AppResult<std::collections::BTreeMap<(u64, usize), werc_domain::asr::Transcript>> {
        let mut result = std::collections::BTreeMap::new();
        let mut before = None;
        let mut size = 0usize;
        loop {
            lease.check()?;
            let page = self.library_page_local(account, "transcript", before)?;
            for item in page.data {
                lease.check()?;
                if let Some(source) = item
                    .entry
                    .source
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<werc_domain::asr::AudioSource>(s).ok())
                {
                    if let werc_domain::asr::AudioSource::Archived {
                        capture_id,
                        page,
                        row,
                    } = source
                    {
                        if capture_id == id {
                            if let Some(mut transcript) = item.entry.transcript {
                                // The user's corrected text is authoritative. Old sentence
                                // timings are not reused for a changed whole transcript.
                                if transcript.text != item.entry.text {
                                    transcript.text = item.entry.text;
                                    transcript.segments.clear();
                                }
                                size += transcript.text.len();
                                if size > 16 * 1024 * 1024 {
                                    return Err(AppError::new(
                                        "EXPORT_TOO_LARGE",
                                        "转写文字超过16MiB，请分批导出",
                                    ));
                                }
                                result.entry((page, row)).or_insert(transcript);
                            }
                        }
                    }
                }
            }
            before = page.next_cursor;
            if before.is_none() {
                break;
            }
        }
        Ok(result)
    }
    pub(super) fn export_capture_interchange(
        &self,
        account: &str,
        id: &str,
        format: &str,
        options: &CaptureExportOptions,
        request: &str,
        out: &mut dyn Write,
    ) -> AppResult<CaptureExportSummary> {
        let lease = self.browse_lease(request)?.for_local_archive();
        let mut rows = Vec::new();
        let mut positions = Vec::new();
        let mut total = 0usize;
        let mut revisions = std::collections::BTreeMap::new();
        let (complete, _, _) = self.visit_capture_positions_checked(
            account,
            id,
            &options.filter,
            || lease.check(),
            |page, position, row| {
                if !revisions.contains_key(&page) {
                    revisions.insert(
                        page,
                        self.repository
                            .library_get(account, &format!("{id}:{page}"))?
                            .header
                            .revision,
                    );
                }
                total += serde_json::to_vec(&row)
                    .map_err(|_| AppError::internal())?
                    .len();
                if rows.len() >= 100000 || total > 64 * 1024 * 1024 {
                    return Err(AppError::new(
                        "EXPORT_TOO_LARGE",
                        "此格式本次超过10万条或64MiB，请缩小范围",
                    ));
                }
                positions.push((page, position));
                rows.push(row);
                Ok(())
            },
        )?;
        let mut ordered = rows.into_iter().zip(positions).collect::<Vec<_>>();
        ordered.sort_by(|(a, _), (b, _)| {
            a.message
                .timestamp
                .cmp(&b.message.timestamp)
                .then(a.source_index.cmp(&b.source_index))
                .then(a.message.local_id.cmp(&b.message.local_id))
        });
        let (mut rows, positions): (Vec<_>, Vec<_>) = ordered.into_iter().unzip();
        let mut media_written = 0;
        let names = self.capture_names_local(account, id)?;
        let login = self.capture_login_local(account, id)?;
        let mut salt = [0u8; 32];
        getrandom::fill(&mut salt).map_err(|_| AppError::internal())?;
        let alias = |value: &str| {
            let mut h = Sha256::new();
            h.update(salt);
            h.update(value.as_bytes());
            format!("匿名-{}", &hex::encode(h.finalize())[..12])
        };
        let self_id = if options.redact_identifiers {
            alias(&login)
        } else {
            login
        };
        let groups = rows
            .iter()
            .filter(|r| werc_domain::live::is_group_account(&r.username))
            .map(|r| {
                if options.redact_identifiers {
                    alias(&r.username)
                } else {
                    r.username.clone()
                }
            })
            .collect::<HashSet<_>>();
        for (index, row) in rows.iter_mut().enumerate() {
            if options.redact_identifiers {
                row.username = alias(&row.username);
                row.message.sender_username = row.message.sender_username.as_deref().map(alias);
                row.message.server_id = None;
                row.message.local_id = (index + 1).to_string();
            }
            if options.omit_text {
                row.message.text = None;
            } else if let Some(text) = &mut row.message.text {
                for (a, b) in &options.replacements {
                    *text = text.replace(a, b);
                }
            }
        }
        let date = |t: i64| {
            chrono::DateTime::from_timestamp(t, 0)
                .map(|v| v.to_rfc3339())
                .unwrap_or_default()
        };
        if format == "xlsx" {
            let columns = if options.xlsx_columns.is_empty() {
                COLUMNS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            } else {
                options.xlsx_columns.clone()
            };
            let labels = columns
                .iter()
                .map(|s| match s.as_str() {
                    "sequence" => "序号",
                    "time" => "时间",
                    "conversation" => "会话",
                    "sender" => "发送者",
                    "local_id" => "本地ID",
                    "server_id" => "服务器ID",
                    "type" => "类型",
                    "text" => "内容",
                    "nickname" => "昵称",
                    "remark" => "备注",
                    "group_nickname" => "群内昵称",
                    "attachment" => "附件",
                    "transcript" => "转写",
                    _ => "本地状态",
                })
                .collect::<Vec<_>>();
            let transcripts = if options.include_transcripts
                && !options.redact_identifiers
                && !options.omit_text
                && options.replacements.is_empty()
            {
                self.capture_export_transcripts(account, id, &lease)?
            } else {
                Default::default()
            };
            let mut attachment_names = std::collections::BTreeMap::new();
            if columns.iter().any(|c| c == "attachment") {
                for (page, row) in &positions {
                    lease.check()?;
                    if !attachment_names.contains_key(page) {
                        attachment_names
                            .insert(*page, self.capture_media_page_local(account, id, *page)?);
                    }
                    let _ = row;
                }
            }
            let bytes = crate::directory_export::xlsx::table(
                "聊天记录",
                &labels,
                rows.iter().enumerate().map(|(i, r)| {
                    columns
                        .iter()
                        .map(|c| match c.as_str() {
                            "sequence" => (i + 1).to_string(),
                            "time" => date(r.message.timestamp),
                            "conversation" => r.username.clone(),
                            "sender" => names.display(
                                &r.username,
                                r.message.sender_username.as_deref().unwrap_or(""),
                                &options.display_name,
                            ),
                            "local_id" => r.message.local_id.clone(),
                            "server_id" => r.message.server_id.clone().unwrap_or_default(),
                            "type" => r.message.message_type.clone(),
                            "text" => r.message.text.clone().unwrap_or_default(),
                            "nickname" | "remark" => {
                                if options.redact_identifiers {
                                    String::new()
                                } else {
                                    names
                                        .contacts
                                        .get(r.message.sender_username.as_deref().unwrap_or(""))
                                        .and_then(|n| {
                                            if c == "nickname" {
                                                n.nickname.clone()
                                            } else {
                                                n.remark.clone()
                                            }
                                        })
                                        .unwrap_or_default()
                                }
                            }
                            "group_nickname" => {
                                if options.redact_identifiers {
                                    String::new()
                                } else {
                                    names
                                        .groups
                                        .get(&r.username)
                                        .and_then(|g| {
                                            g.get(
                                                r.message.sender_username.as_deref().unwrap_or(""),
                                            )
                                        })
                                        .cloned()
                                        .unwrap_or_default()
                                }
                            }
                            "attachment" => {
                                if options.redact_identifiers
                                    || options.omit_text
                                    || !options.replacements.is_empty()
                                {
                                    String::new()
                                } else {
                                    attachment_names
                                        .get(&positions[i].0)
                                        .and_then(|list| list.get(positions[i].1))
                                        .filter(|v| v.matches_message)
                                        .and_then(|v| v.saved.as_ref())
                                        .map(|m| m.name.clone())
                                        .unwrap_or_default()
                                }
                            }
                            "transcript" => {
                                if r.message.content_status == "edited_local" {
                                    String::new()
                                } else {
                                    transcripts
                                        .get(&positions[i])
                                        .map(|t| t.text.clone())
                                        .unwrap_or_default()
                                }
                            }
                            _ => r.message.content_status.clone(),
                        })
                        .collect()
                }),
                &[
                    "所选本机已保存资料，所有ID和内容按文本保存，不执行公式。",
                    "未保存的正文或发送者保持空值；不代表服务器完整历史。",
                ],
            )?;
            out.write_all(&bytes).map_err(|_| failed())?;
        } else if format == "sql" {
            out.write_all(b"BEGIN TRANSACTION;\nCREATE TABLE werc_messages (sequence INTEGER,conversation TEXT,sender TEXT,local_id TEXT,server_id TEXT,timestamp INTEGER,message_type TEXT,content TEXT,local_status TEXT);\n").map_err(|_|failed())?;
            for (i, r) in rows.iter().enumerate() {
                lease.check()?;
                let m = &r.message;
                writeln!(
                    out,
                    "INSERT INTO werc_messages VALUES ({},{},{},{},{},{},{},{},{});",
                    i + 1,
                    sql(&r.username),
                    m.sender_username
                        .as_deref()
                        .map(sql)
                        .unwrap_or_else(|| "NULL".into()),
                    sql(&m.local_id),
                    m.server_id
                        .as_deref()
                        .map(sql)
                        .unwrap_or_else(|| "NULL".into()),
                    m.timestamp,
                    sql(&m.message_type),
                    m.text.as_deref().map(sql).unwrap_or_else(|| "NULL".into()),
                    sql(&m.content_status)
                )
                .map_err(|_| failed())?;
            }
            out.write_all(b"COMMIT;\n").map_err(|_| failed())?;
        } else if format == "weclone_csv" {
            if rows.iter().any(|r| r.message.sender_username.is_none()) {
                return Err(AppError::new(
                    "EXPORT_SENDER_UNKNOWN",
                    "WeClone需要明确发送方向；所选记录存在未知发送者，请先筛选身份完整的资料",
                ));
            }
            // UTC makes the file independent of the machine that imports it.
            out.write_all(
                b"id,MsgSvrID,type_name,is_sender,talker,room_name,msg,src,CreateTime\r\n",
            )
            .map_err(|_| failed())?;
            for (i, r) in rows.iter().enumerate() {
                lease.check()?;
                let m = &r.message;
                let kind = match row_message_type(r) {
                    0 | 25 => "文本",
                    1 => "图片",
                    2 => "语音",
                    3 => "视频",
                    5 => "动画表情",
                    8 => "位置",
                    23 => "通话",
                    80 => "系统",
                    _ => "其他",
                };
                let time = chrono::DateTime::from_timestamp(m.timestamp, 0)
                    .map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                let fields = [
                    (i + 1).to_string(),
                    m.server_id.clone().unwrap_or_default(),
                    kind.into(),
                    if m.sender_username.as_deref() == Some(self_id.as_str()) {
                        "1".into()
                    } else {
                        "0".into()
                    },
                    m.sender_username.clone().unwrap_or_default(),
                    if groups.contains(&r.username) {
                        r.username.clone()
                    } else {
                        String::new()
                    },
                    m.text.clone().unwrap_or_default(),
                    String::new(),
                    time,
                ];
                writeln!(
                    out,
                    "{}\r",
                    fields.iter().map(|s| csv(s)).collect::<Vec<_>>().join(",")
                )
                .map_err(|_| failed())?;
            }
        } else {
            let conversations = rows
                .iter()
                .map(|r| r.username.as_str())
                .collect::<HashSet<_>>();
            if conversations.len() > 1 {
                return Err(AppError::new(
                    "EXPORT_SINGLE_CONVERSATION_REQUIRED",
                    "ChatLab单文件只包含一个会话，请先指定一个会话账号",
                ));
            }
            let name = conversations
                .iter()
                .next()
                .copied()
                .or_else(|| options.filter.conversations.first().map(String::as_str))
                .unwrap_or("空会话");
            let fallback;
            let name = if options.redact_identifiers && rows.is_empty() && name != "空会话" {
                fallback = alias(name);
                fallback.as_str()
            } else {
                name
            };
            let group = groups.contains(name);
            let meta = json!({"name":name,"platform":"wechat","type":if group{"group"}else{"private"},"ownerId":self_id});
            let header = json!({"version":"0.0.2","exportedAt":chrono::Utc::now().timestamp(),"generator":"WeRC"});
            let members=rows.iter().map(|r|r.message.sender_username.as_deref().unwrap_or("werc:unknown")).chain(std::iter::once(self_id.as_str())).collect::<std::collections::BTreeSet<_>>().into_iter().map(|id|json!({"platformId":id,"accountName":if id=="werc:unknown"{"身份未知".into()}else{names.display(name,id,&options.display_name)}})).collect::<Vec<_>>();
            let messages=rows.iter().map(|r|json!({"sender":r.message.sender_username.as_deref().unwrap_or("werc:unknown"),"accountName":names.display(&r.username,r.message.sender_username.as_deref().unwrap_or("身份未知"),&options.display_name),"timestamp":r.message.timestamp,"type":row_message_type(r),"content":r.message.text})).collect::<Vec<_>>();
            if format == "chatlab_zip" {
                use std::io::Cursor;
                use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};
                let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
                let settings =
                    SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
                let mut manifest = Vec::new();
                let mut bytes_total = 0usize;
                let mut media_pages = std::collections::BTreeMap::new();
                let suppress = options.redact_identifiers
                    || options.omit_text
                    || !options.replacements.is_empty();
                let transcripts = if options.include_transcripts && !suppress {
                    self.capture_export_transcripts(account, id, &lease)?
                } else {
                    Default::default()
                };
                for (index, (page, row)) in positions.iter().enumerate() {
                    lease.check()?;
                    if !media_pages.contains_key(page) {
                        media_pages
                            .insert(*page, self.capture_media_page_local(account, id, *page)?);
                    }
                    let media = media_pages
                        .get(page)
                        .and_then(|list| list.get(*row))
                        .ok_or_else(AppError::internal)?;
                    if media.kind.is_none() && media.saved.is_none() {
                        continue;
                    }
                    let kind = media
                        .kind
                        .as_ref()
                        .or_else(|| media.saved.as_ref().map(|s| &s.kind))
                        .map(|k| match k {
                            crate::conversation_media::ConversationMediaKind::Image => "image",
                            crate::conversation_media::ConversationMediaKind::Voice => "voice",
                            crate::conversation_media::ConversationMediaKind::Video => "video",
                            crate::conversation_media::ConversationMediaKind::Emoticon => {
                                "emoticon"
                            }
                            crate::conversation_media::ConversationMediaKind::File => "file",
                        });
                    if !options.media_kinds.is_empty()
                        && kind.is_some_and(|k| !options.media_kinds.iter().any(|v| v == k))
                    {
                        manifest
                            .push(json!({"messageIndex":index,"status":"excluded_by_selection"}));
                        continue;
                    }
                    let mut entry = json!({"messageIndex":index,"status":if suppress{"omitted_for_redaction"}else if !media.matches_message{"original_message_changed"}else{"not_cached"}});
                    if !suppress && media.matches_message {
                        if let Some(saved) = &media.saved {
                            let mut bytes = zeroize::Zeroizing::new(Vec::new());
                            self.export_capture_media_local(
                                account,
                                id,
                                *page,
                                *row,
                                &uuid::Uuid::new_v4().to_string(),
                                &mut *bytes,
                            )?;
                            bytes_total = bytes_total.saturating_add(bytes.len());
                            if bytes_total > 48 * 1024 * 1024 {
                                return Err(AppError::new(
                                    "EXPORT_TOO_LARGE",
                                    "附件超过48MiB，请缩小范围或使用媒体归档ZIP",
                                ));
                            }
                            let ext = match saved.mime.as_str() {
                                "image/png" => "png",
                                "image/gif" => "gif",
                                "audio/wav" => "wav",
                                "video/mp4" => "mp4",
                                _ => "bin",
                            };
                            let path = format!("media/{index}.{ext}");
                            zip.start_file(&path, settings).map_err(|_| failed())?;
                            zip.write_all(&bytes).map_err(|_| failed())?;
                            entry = json!({"messageIndex":index,"status":"saved","path":path,"name":saved.name,"mime":saved.mime,"size":saved.size,"sha256":saved.sha256});
                            media_written += 1;
                        }
                    }
                    if !suppress
                        && media.matches_message
                        && rows[index].message.content_status != "edited_local"
                    {
                        if let Some(transcript) = transcripts.get(&(*page, *row)) {
                            let path = format!("transcripts/{index}.txt");
                            zip.start_file(&path, settings).map_err(|_| failed())?;
                            zip.write_all(transcript.text.as_bytes())
                                .map_err(|_| failed())?;
                            entry["transcript"] = json!({"textPath":path});
                            if !transcript.segments.is_empty() {
                                let srt = format!("transcripts/{index}.srt");
                                zip.start_file(&srt, settings).map_err(|_| failed())?;
                                zip.write_all(transcript.srt().as_bytes())
                                    .map_err(|_| failed())?;
                                entry["transcript"]["srtPath"] = json!(srt);
                            }
                        }
                    }
                    manifest.push(entry);
                }
                let mut avatars = Vec::new();
                if options.include_avatars && !suppress {
                    for (index, username) in rows
                        .iter()
                        .filter_map(|r| r.message.sender_username.as_deref())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .enumerate()
                    {
                        lease.check()?;
                        let Some(url) = names
                            .contacts
                            .get(username)
                            .and_then(|c| c.avatar_url.as_deref())
                        else {
                            avatars.push(json!({"username":username,"status":"not_cached"}));
                            continue;
                        };
                        if let Some(avatar) = self.cached_contact_avatar(account, username, url)? {
                            let bytes = avatar.bytes()?;
                            bytes_total += bytes.len();
                            if bytes_total > 48 * 1024 * 1024 {
                                return Err(AppError::new(
                                    "EXPORT_TOO_LARGE",
                                    "附件及头像超过48MiB",
                                ));
                            }
                            let path = format!("avatars/{index}.png");
                            zip.start_file(&path, settings).map_err(|_| failed())?;
                            zip.write_all(&bytes).map_err(|_| failed())?;
                            avatars.push(json!({"username":username,"path":path,"status":"saved","sha256":avatar.sha256}));
                        } else {
                            avatars.push(json!({"username":username,"status":"not_cached"}));
                        }
                    }
                }
                zip.start_file("chatlab.json", settings)
                    .map_err(|_| failed())?;
                serde_json::to_writer(
                    &mut zip,
                    &json!({"chatlab":header,"meta":meta,"members":members,"messages":messages}),
                )
                .map_err(|_| failed())?;
                zip.start_file("attachments.json", settings)
                    .map_err(|_| failed())?;
                serde_json::to_writer(&mut zip,&json!({"format":"werc-chatlab-attachments-1","indexBase":0,"attachments":manifest,"avatars":avatars})).map_err(|_|failed())?;
                zip.start_file("README.txt", settings)
                    .map_err(|_| failed())?;
                zip.write_all("先解压全部文件。将chatlab.json导入ChatLab；attachments.json是WeRC附件索引，messageIndex从0起对应messages顺序，path仅含相对路径。附件索引不属于ChatLab标准，不假设其自动读取。只包含本机已保存且仍匹配的媒体，不联网补齐。脱敏时不导出二进制附件。\n".as_bytes()).map_err(|_|failed())?;
                let bytes = zip.finish().map_err(|_| failed())?.into_inner();
                out.write_all(&bytes).map_err(|_| failed())?;
            } else if format == "chatlab_json" {
                serde_json::to_writer(
                    &mut *out,
                    &json!({"chatlab":header,"meta":meta,"members":members,"messages":messages}),
                )
                .map_err(|_| failed())?;
            } else {
                let mut first = json!({"_type":"header","chatlab":header,"meta":meta});
                write_json_line(out, &mut first)?;
                for mut member in members {
                    member["_type"] = json!("member");
                    write_json_line(out, &mut member)?;
                }
                for mut message in messages {
                    message["_type"] = json!("message");
                    write_json_line(out, &mut message)?;
                }
            }
        }
        lease.check()?;
        for (page, revision) in revisions {
            if self
                .repository
                .library_get(account, &format!("{id}:{page}"))?
                .header
                .revision
                != revision
            {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "导出期间资料已修改，请重新导出",
                ));
            }
        }
        Ok(CaptureExportSummary {
            rows: rows.len() as u64,
            complete_selected_sources: complete,
            redacted: options.redact_identifiers
                || options.omit_text
                || !options.replacements.is_empty(),
            media_written,
        })
    }
}
fn write_json_line(out: &mut dyn Write, value: &mut Value) -> AppResult<()> {
    serde_json::to_writer(&mut *out, value).map_err(|_| failed())?;
    out.write_all(b"\n").map_err(|_| failed())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chatlab_standard_types_preserve_distinct_interactions() {
        for (native, expected) in [
            (1, 0),
            (3, 1),
            (34, 2),
            (43, 3),
            (47, 5),
            (48, 8),
            (42, 27),
            (50, 23),
            (10000, 80),
            (10002, 81),
        ] {
            assert_eq!(message_type(&native.to_string()), expected);
        }
        for (subtype, expected) in [
            (6u64, 4),
            (5, 7),
            (19, 26),
            (33, 24),
            (57, 25),
            (2000, 21),
            (2001, 20),
        ] {
            assert_eq!(message_type(&((subtype << 32) | 49).to_string()), expected);
        }
        assert_eq!(message_type("49"), 99);
        assert_eq!(message_type("unknown"), 99);
    }
}
