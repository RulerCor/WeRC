//! Specialized record browsing over encrypted saved pages, available offline.
use super::*;
use std::io::Write;
use werc_domain::message_card::records::{RecordDetail, RecordKind};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordQuery {
    pub kind: Option<RecordKind>,
    pub direction: Option<RecordDirection>,
    pub raw_status: Option<String>,
    #[serde(default)]
    pub selected: Vec<MediaPosition>,
    pub username: Option<String>,
    pub from: Option<i64>,
    pub until: Option<i64>,
    #[serde(default)]
    pub text: String,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordDirection {
    Incoming,
    Outgoing,
    Unknown,
}
#[derive(Serialize)]
pub struct RecordItem {
    pub position: MediaPosition,
    pub username: String,
    pub sender: Option<String>,
    pub server_id: Option<String>,
    pub timestamp: i64,
    pub local_status: String,
    pub local_id: String,
    pub message_type: String,
    pub direction: RecordDirection,
    pub detail: RecordDetail,
}
#[derive(Serialize)]
pub struct RecordCatalog {
    pub data: Vec<RecordItem>,
    pub next: Option<MediaPosition>,
    pub scanned: usize,
    pub unreadable: usize,
    #[serde(skip)]
    revisions: std::collections::BTreeMap<String, i64>,
}
#[derive(Serialize)]
pub struct MessageLocation {
    pub position: MediaPosition,
    pub copies: u64,
    pub complete_selected_sources: bool,
}
fn server_key(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().filter(|v| *v > 0 && v.to_string() == value)
        .or_else(|| value.parse::<i64>().ok().filter(|v| *v < 0 && v.to_string() == value).map(|v| v as u64))
}
impl Application {
    /// Locate only by exact conversation and server identity. Multiple identical
    /// shard copies are harmless; conflicting bodies are not guessed by date/text.
    pub fn capture_message_location_local(&self, account:&str, id:&str, username:&str, server:&str, request_id:&str) -> AppResult<MessageLocation> {
        use sha2::{Digest, Sha256};
        if !werc_domain::live::valid_live_username(username) || server_key(server).is_none() {
            return Err(AppError::new("INVALID_ARGUMENT", "请提供精确会话账号和非零服务端消息编号"));
        }
        let header=self.repository.library_get(account,id)?.header;
        let pages=self.source_capture_status_local(account,id)?.pages;
        let mut found=None;
        let mut digest=None;
        let mut copies=0u64;
        let filter=crate::capture_tools::CaptureFilter { conversations:vec![username.into()], ..Default::default() };
        let (complete,_,_)=self.visit_capture_positions_local(account,id,&filter,request_id,|page,row,record| {
            let m=&record.message;
            if m.server_id.as_deref().and_then(server_key)!=server_key(server) {return Ok(());}
            let signature=Sha256::digest(serde_json::to_vec(&(&m.message_type,m.timestamp,&m.sender_username,&m.text,&m.metadata,&m.content_status)).map_err(|_|AppError::internal())?);
            if digest.as_ref().is_some_and(|d|d!=&signature) {return Err(AppError::new("MESSAGE_ID_AMBIGUOUS", "相同会话和消息编号的副本内容不一致，请检查来源"));}
            digest=Some(signature);
            copies+=1;
            if found.is_none(){found=Some(MediaPosition{page,row});}
            Ok(())
        })?;
        if self.repository.library_get(account,id)?.header.revision!=header.revision || self.source_capture_status_local(account,id)?.pages!=pages {
            return Err(AppError::new("DOCUMENT_CONFLICT", "查找期间归档已变化，请重新定位"));
        }
        Ok(MessageLocation {position:found.ok_or_else(||AppError::new("MESSAGE_NOT_IN_CAPTURE", "所选归档未保存这条原消息；可补读对应来源后重试"))?, copies, complete_selected_sources:complete})
    }
    pub fn capture_records_local(
        &self,
        account: &str,
        id: &str,
        query: &RecordQuery,
        after: Option<MediaPosition>,
        request_id: &str,
    ) -> AppResult<RecordCatalog> {
        if query.selected.len() > 1000
            || query
                .raw_status
                .as_ref()
                .is_some_and(|s| s.len() > 64 || s.chars().any(char::is_control))
            || query.text.len() > 512
            || query.text.contains('\0')
            || query
                .username
                .as_deref()
                .is_some_and(|u| !werc_domain::live::valid_live_username(u))
            || matches!((query.from, query.until),(Some(a),Some(b)) if a>=b)
        {
            return Err(AppError::new("INVALID_ARGUMENT", "记录筛选条件无效"));
        }
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let (capture_header, _): (_, Capture) =
            self.open_document(account, id, "source_capture")?;
        let status = self.source_capture_status_local(account, id)?;
        let login = self.capture_login_local(account, id)?;
        let mut pos = after.unwrap_or(MediaPosition { page: 0, row: 0 });
        let mut result = RecordCatalog {
            data: vec![],
            next: None,
            scanned: 0,
            unreadable: 0,
            revisions: std::collections::BTreeMap::from([(id.to_owned(), capture_header.revision)]),
        };
        let needle = query.text.to_lowercase();
        while pos.page < status.pages && result.scanned < 1000 && result.data.len() < 40 {
            lease.check()?;
            let (header, batch): (_, CaptureBatch) =
                self.open_document(account, &format!("{id}:{}", pos.page), "capture_page")?;
            result
                .revisions
                .insert(format!("{id}:{}", pos.page), header.revision);
            if batch.capture_id != id || batch.index != pos.page {
                return Err(AppError::new("DOCUMENT_CORRUPTED", "归档页归属无效"));
            }
            if pos.row > batch.rows.len() {
                return Err(AppError::new("INVALID_ARGUMENT", "记录续读位置无效"));
            }
            for (index, row) in batch.rows.iter().enumerate().skip(pos.row) {
                lease.check()?;
                if result.scanned >= 1000 {
                    break;
                }
                let position = MediaPosition {
                    page: pos.page,
                    row: index,
                };
                pos.row = index + 1;
                result.scanned += 1;
                if !query.selected.is_empty()
                    && !query
                        .selected
                        .iter()
                        .any(|p| p.page == position.page && p.row == position.row)
                {
                    continue;
                }
                let m = &row.message;
                let direction = match m.sender_username.as_deref() {
                    Some(sender) if !login.is_empty() && sender == login => {
                        RecordDirection::Outgoing
                    }
                    Some(sender)
                        if !login.is_empty()
                            && !sender.is_empty()
                            && (werc_domain::live::is_group_account(&row.username)
                                || sender == row.username) =>
                    {
                        RecordDirection::Incoming
                    }
                    _ => RecordDirection::Unknown,
                };
                if query.direction.is_some_and(|d| d != direction) {
                    continue;
                }
                if m.content_status == "deleted_local"
                    || query.username.as_ref().is_some_and(|u| u != &row.username)
                    || query.from.is_some_and(|v| m.timestamp < v)
                    || query.until.is_some_and(|v| m.timestamp >= v)
                {
                    continue;
                }
                let kind = (m.message_type.parse::<i64>().unwrap_or(0) & 0xffffffff) as u32;
                if !matches!(kind, 37 | 42 | 48 | 49 | 50 | 10000 | 10002) {
                    continue;
                }
                let body = m.metadata_body().or(m.text.as_deref()).unwrap_or("");
                let detail = match werc_domain::message_card::records::extract(kind, body) {
                    Ok(Some(detail)) => detail,
                    Ok(None) => continue,
                    Err(_) => {
                        result.unreadable += 1;
                        continue;
                    }
                };
                if query
                    .raw_status
                    .as_ref()
                    .is_some_and(|s| detail.fields.get("原始状态码") != Some(s))
                {
                    continue;
                }
                if query.kind.is_some_and(|k| k != detail.kind) {
                    continue;
                }
                if !needle.is_empty()
                    && !format!(
                        "{}\n{}\n{}\n{}",
                        detail.title,
                        detail.text,
                        detail
                            .fields
                            .values()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n"),
                        detail.url.as_deref().unwrap_or("")
                    )
                    .to_lowercase()
                    .contains(&needle)
                {
                    continue;
                }
                result.data.push(RecordItem {
                    position,
                    username: row.username.clone(),
                    sender: m.sender_username.clone(),
                    server_id: m.server_id.clone(),
                    timestamp: m.timestamp,
                    local_status: m.content_status.clone(),
                    local_id: m.local_id.clone(),
                    message_type: m.message_type.clone(),
                    direction,
                    detail,
                });
                if result.data.len() >= 40 || result.scanned >= 1000 {
                    break;
                }
            }
            if pos.row >= batch.rows.len() {
                pos.page += 1;
                pos.row = 0;
            }
        }
        result.next = (pos.page < status.pages).then_some(pos);
        Ok(result)
    }
    pub fn export_capture_records_local(
        &self,
        account: &str,
        id: &str,
        query: &RecordQuery,
        format: &str,
        request_id: &str,
        out: &mut dyn Write,
    ) -> AppResult<u64> {
        if !matches!(format, "csv" | "jsonl" | "json" | "xlsx") {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "记录导出支持CSV、JSONL、JSON、XLSX",
            ));
        }
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let failed = || AppError::new("EXPORT_WRITE_FAILED", "记录导出写入失败");
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let mut sheets = std::collections::BTreeMap::<String, (usize, u32)>::new();
        let mut long_text = Vec::<[String; 5]>::new();
        if format == "csv" {
            out.write_all(b"\xef\xbb\xbf").map_err(|_| failed())?;
            writeln!(out, "{}", RECORD_HEADERS.join(",")).map_err(|_| failed())?;
        }
        if format == "json" {
            out.write_all(b"{\"schema_version\":1,\"data_mode\":\"local_archive\",\"records\":[")
                .map_err(|_| failed())?;
        }
        let mut after = None;
        let mut revisions = std::collections::BTreeMap::new();
        let mut count = 0u64;
        let mut xlsx_bytes = 0usize;
        loop {
            lease.check()?;
            let page = self.capture_records_local(
                account,
                id,
                query,
                after,
                &uuid::Uuid::new_v4().to_string(),
            )?;
            for (document, revision) in page.revisions {
                if revisions
                    .insert(document, revision)
                    .is_some_and(|old| old != revision)
                {
                    return Err(AppError::new(
                        "DOCUMENT_CONFLICT",
                        "导出期间资料已修改，请重新导出",
                    ));
                }
            }
            for item in page.data {
                lease.check()?;
                match format {
                    "jsonl" | "json" => {
                        if format == "json" && count > 0 {
                            out.write_all(b",").map_err(|_| failed())?;
                        }
                        serde_json::to_writer(&mut *out, &item).map_err(|_| failed())?;
                        if format == "jsonl" {
                            out.write_all(b"\n").map_err(|_| failed())?;
                        }
                    }
                    "csv" => {
                        let values = record_fields(&item)?;
                        let line = values
                            .iter()
                            .enumerate()
                            .map(|(i, s)| {
                                let prefix = if s.trim_start().starts_with(['=', '+', '-', '@'])
                                    || s.starts_with(['\t', '\r'])
                                    || (matches!(i, 8 | 11)
                                        && s.len() > 15
                                        && s.bytes().all(|c| c.is_ascii_digit()))
                                {
                                    "'"
                                } else {
                                    ""
                                };
                                format!("\"{}{}\"", prefix, s.replace('"', "\"\""))
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        writeln!(out, "{line}").map_err(|_| failed())?;
                    }
                    "xlsx" => {
                        let values = record_fields(&item)?;
                        xlsx_bytes += values.iter().map(String::len).sum::<usize>();
                        if count >= 50000 || xlsx_bytes > 64 * 1024 * 1024 {
                            return Err(AppError::new(
                                "EXPORT_LIMIT",
                                "Excel最多5万条或64MiB文本，请缩小范围或使用JSONL",
                            ));
                        }
                        let name = record_category(item.detail.kind).to_owned();
                        if !sheets.contains_key(&name) {
                            let index = sheets.len();
                            let sheet = workbook.add_worksheet();
                            sheet.set_name(&name).map_err(|_| failed())?;
                            for (col, title) in RECORD_HEADERS.iter().enumerate() {
                                sheet
                                    .write_string(0, col as u16, *title)
                                    .map_err(|_| failed())?;
                            }
                            sheet.set_freeze_panes(1, 0).map_err(|_| failed())?;
                            sheet
                                .set_column_range_width(0, 14, 24)
                                .map_err(|_| failed())?;
                            sheets.insert(name.clone(), (index, 1));
                        }
                        let (index, row) = sheets.get_mut(&name).ok_or_else(failed)?;
                        let sheet = workbook
                            .worksheet_from_index(*index)
                            .map_err(|_| failed())?;
                        for (col, value) in values.iter().enumerate() {
                            // Explicit string cells preserve long IDs and formula-like user text.
                            if value.encode_utf16().count() <= 32767 {
                                sheet
                                    .write_string(*row, col as u16, value)
                                    .map_err(|_| failed())?;
                            } else {
                                sheet
                                    .write_string(*row, col as u16, "完整内容见“长文本”工作表")
                                    .map_err(|_| failed())?;
                                let mut chunk = String::new();
                                let mut units = 0;
                                let mut part = 1;
                                for c in value.chars() {
                                    if units + c.len_utf16() > 30000 {
                                        long_text.push([
                                            name.clone(),
                                            (*row + 1).to_string(),
                                            RECORD_HEADERS[col].into(),
                                            part.to_string(),
                                            std::mem::take(&mut chunk),
                                        ]);
                                        units = 0;
                                        part += 1;
                                    }
                                    units += c.len_utf16();
                                    chunk.push(c);
                                }
                                if !chunk.is_empty() {
                                    long_text.push([
                                        name.clone(),
                                        (*row + 1).to_string(),
                                        RECORD_HEADERS[col].into(),
                                        part.to_string(),
                                        chunk,
                                    ]);
                                }
                            }
                        }
                        *row += 1;
                    }
                    _ => unreachable!(),
                }
                count += 1;
            }
            after = page.next;
            if after.is_none() {
                break;
            }
        }
        if format == "json" {
            write!(out, "],\"count\":{count},\"historical_complete\":false}}")
                .map_err(|_| failed())?;
        }
        if format == "xlsx" {
            if !long_text.is_empty() {
                let sheet = workbook.add_worksheet();
                sheet.set_name("长文本").map_err(|_| failed())?;
                for (col, name) in ["工作表", "Excel行号", "字段", "分段", "内容"]
                    .iter()
                    .enumerate()
                {
                    sheet
                        .write_string(0, col as u16, *name)
                        .map_err(|_| failed())?;
                }
                for (row, values) in long_text.iter().enumerate() {
                    lease.check()?;
                    for (col, value) in values.iter().enumerate() {
                        sheet
                            .write_string(row as u32 + 1, col as u16, value)
                            .map_err(|_| failed())?;
                    }
                }
            }
            let info = workbook.add_worksheet();
            info.set_name("说明").map_err(|_| failed())?;
            info.write_string(
                0,
                0,
                "本地聊天归档分类记录；不代表完整账单或服务端状态。方向为此消息的发送方向。",
            )
            .map_err(|_| failed())?;
            info.write_string(1,0,format!("共{count}条；完整编号与用户文字作为字符串保存。长内容按分段顺序拼接，无分隔符。更新时间以来源为准。")).map_err(|_|failed())?;
            let bytes = zeroize::Zeroizing::new(workbook.save_to_buffer().map_err(|_| failed())?);
            lease.check()?;
            out.write_all(&bytes).map_err(|_| failed())?;
        }
        lease.check()?;
        for (document, revision) in revisions {
            lease.check()?;
            if self
                .repository
                .library_get(account, &document)?
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
        Ok(count)
    }
}
const RECORD_HEADERS: [&str; 15] = [
    "会话",
    "发送者",
    "时间",
    "类别",
    "标题",
    "正文",
    "详情",
    "链接",
    "服务器编号",
    "本地状态",
    "消息方向",
    "本地编号",
    "消息类型",
    "归档页",
    "页内行",
];
fn record_category(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Transfer => "转账",
        RecordKind::RedPacket => "红包",
        RecordKind::FriendRequest => "好友验证",
        RecordKind::Call => "通话",
        RecordKind::Revoke => "撤回通知",
        RecordKind::Link => "链接",
        RecordKind::Contact => "名片",
        RecordKind::Location => "位置",
        RecordKind::Forward => "合并转发",
        RecordKind::Quote => "引用",
        RecordKind::Share => "分享",
        RecordKind::System => "系统记录",
    }
}
fn record_fields(item: &RecordItem) -> AppResult<[String; 15]> {
    Ok([
        item.username.clone(),
        item.sender.clone().unwrap_or_default(),
        item.timestamp.to_string(),
        record_category(item.detail.kind).into(),
        item.detail.title.clone(),
        item.detail.text.clone(),
        serde_json::to_string(&item.detail.fields).map_err(|_| AppError::internal())?,
        item.detail.url.clone().unwrap_or_default(),
        item.server_id.clone().unwrap_or_default(),
        item.local_status.clone(),
        match item.direction {
            RecordDirection::Incoming => "接收",
            RecordDirection::Outgoing => "发送",
            RecordDirection::Unknown => "未知",
        }
        .into(),
        item.local_id.clone(),
        item.message_type.clone(),
        item.position.page.to_string(),
        item.position.row.to_string(),
    ])
}
