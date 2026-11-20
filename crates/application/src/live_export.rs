//! Export a complete selected range without replacing the active conversation view.
use crate::Application;
mod readable;
use serde::{Deserialize, Serialize};
use std::io::Write;
use werc_domain::{AppError, AppResult, live::*};
use zeroize::Zeroizing;
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveExportFormat {
    Json,
    Csv,
    Txt,
    Xlsx,
    Html,
    Md,
}
impl LiveExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Txt => "txt",
            Self::Xlsx => "xlsx",
            Self::Html => "html",
            Self::Md => "md",
        }
    }
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Txt => "text/plain; charset=utf-8",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Html => "text/html; charset=utf-8",
            Self::Md => "text/markdown; charset=utf-8",
        }
    }
}
const MAX_ROWS: usize = 50_000;
const MAX_BYTES: usize = 32 * 1024 * 1024;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LiveExportSelection {
    pub username: String,
    #[serde(default)]
    pub query: LiveMessageQuery,
}
#[derive(Serialize)]
pub struct LiveExportSummary {
    pub rows: usize,
    pub pages: usize,
    pub bytes: usize,
    pub source_count: usize,
    pub complete_selected_range: bool,
    pub cross_page_snapshot: bool,
    pub media_written: usize,
}
pub struct LiveExport {
    pub bytes: Zeroizing<Vec<u8>>,
    pub summary: LiveExportSummary,
}
#[derive(Serialize)]
struct Row {
    source_index: usize,
    message: LiveMessage,
}
fn limit() -> AppError {
    AppError::new(
        "LIVE_EXPORT_LIMIT",
        "导出超过5万条或32MiB，请缩小日期范围；未写入文件",
    )
}
impl Application {
    pub fn export_live_messages_local<T>(
        &self,
        account: &str,
        view_id: &str,
        selection: &LiveExportSelection,
        format: LiveExportFormat,
        request_id: &str,
        finish: impl FnOnce(LiveExport) -> AppResult<T>,
    ) -> AppResult<T> {
        if !valid_live_username(&selection.username) || !selection.query.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "会话或导出条件无效"));
        }
        let lease = self.source_lease(request_id)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        lease.check()?;
        let (binding, sources) =
            self.with_search_parent(account, view_id, |binding, _, sources, _| {
                Ok((binding.to_owned(), sources.to_vec()))
            })?;
        let keys = self.live_keys_local(account)?;
        let mut rows = Vec::new();
        let mut retained = 0;
        let mut pages = 0;
        for (index, source) in sources.iter().enumerate() {
            let key = crate::keyring::key_for(&keys, &source.salt)?;
            let mut selected = LiveConversationSelection {
                username: selection.username.clone(),
                query: Some(selection.query.clone()),
                before: None,
                include_anchor: false,
                newer_context: false,
            };
            loop {
                lease.check()?;
                let window = match self.provider.read_live_conversation(
                    account,
                    &source.database,
                    &key,
                    &source.fingerprint,
                    &selected,
                    &lease.cancel,
                ) {
                    Ok(window) => window,
                    Err(e)
                        if e.code == "LIVE_CONVERSATION_NOT_IN_SOURCE"
                            && selected.before.is_none() =>
                    {
                        break;
                    }
                    Err(e) => return Err(e),
                };
                pages += 1;
                drop(crate::live_view::encode_window(&window)?);
                if window.rows.iter().any(|r| !selection.query.matches(r)) {
                    return Err(AppError::new(
                        "LIVE_PROTOCOL_INVALID",
                        "来源返回了不符合导出条件的消息",
                    ));
                }
                let previous = selected.before.as_ref().and_then(LiveAnchor::key);
                if window.rows.first().is_some_and(|r| {
                    previous
                        .is_some_and(|p| (r.timestamp, r.local_id.parse::<i64>().unwrap_or(0)) >= p)
                }) {
                    return Err(AppError::new("LIVE_PROTOCOL_INVALID", "导出续页没有前进"));
                }
                selected.before = window.rows.last().map(|r| LiveAnchor {
                    timestamp: r.timestamp,
                    local_id: r.local_id.clone(),
                    server_id: r.server_id.clone(),
                });
                if window.has_older && selected.before.is_none() {
                    return Err(AppError::new(
                        "LIVE_PROTOCOL_INVALID",
                        "导出续页缺少定位信息",
                    ));
                }
                for mut message in window.rows {
                    // Raw XML contains download credentials; public exports retain safe fields only.
                    message.metadata = None;
                    let row = Row {
                        source_index: index + 1,
                        message,
                    };
                    retained +=
                        Zeroizing::new(serde_json::to_vec(&row).map_err(|_| AppError::internal())?)
                            .len();
                    if rows.len() >= MAX_ROWS || retained > MAX_BYTES {
                        return Err(limit());
                    }
                    rows.push(row);
                }
                if !window.has_older {
                    break;
                }
            }
        }
        rows.sort_by(|a, b| {
            (
                a.message.timestamp,
                a.source_index,
                a.message.local_id.parse::<i64>().unwrap_or(0),
            )
                .cmp(&(
                    b.message.timestamp,
                    b.source_index,
                    b.message.local_id.parse::<i64>().unwrap_or(0),
                ))
        });
        lease.check()?;
        let bytes = render(account, selection, &rows, format, sources.len())?;
        self.with_search_parent(account, view_id, |current, _, _, _| {
            lease.check()?;
            if current != binding {
                return Err(AppError::new(
                    "CURSOR_STALE",
                    "导出期间来源或登录身份发生变化",
                ));
            }
            Ok(())
        })?;
        let summary = LiveExportSummary {
            rows: rows.len(),
            pages,
            bytes: bytes.len(),
            source_count: sources.len(),
            complete_selected_range: true,
            cross_page_snapshot: false,
            media_written: 0,
        };
        finish(LiveExport { bytes, summary })
    }
}
fn fields(row: &Row) -> Vec<String> {
    let m = &row.message;
    vec![
        row.source_index.to_string(),
        m.local_id.clone(),
        m.server_id.clone().unwrap_or_default(),
        chrono::DateTime::from_timestamp(m.timestamp, 0)
            .map(|d| d.to_rfc3339())
            .unwrap_or_default(),
        m.timestamp.to_string(),
        m.sender_username.clone().unwrap_or_default(),
        m.message_type.clone(),
        m.content_status.clone(),
        m.text.clone().unwrap_or_default(),
    ]
}
fn render(
    account: &str,
    selection: &LiveExportSelection,
    rows: &[Row],
    format: LiveExportFormat,
    sources: usize,
) -> AppResult<Zeroizing<Vec<u8>>> {
    if matches!(format, LiveExportFormat::Html | LiveExportFormat::Md) {
        return readable::render(selection, rows, format, sources);
    }
    const LABELS: [&str; 9] = [
        "来源序号",
        "本地消息ID",
        "服务器消息ID",
        "时间UTC",
        "Unix时间戳",
        "发送者账号",
        "消息类型",
        "内容状态",
        "文字",
    ];
    if matches!(format, LiveExportFormat::Xlsx) {
        return crate::directory_export::xlsx::table(
            "会话消息",
            &LABELS,
            rows.iter().map(fields),
            &[
                "数据来自所选本地消息分片，按时间正序；来源序号按选择顺序，不合并同ID跨分片记录。",
                "每次续页是独立只读事务，不是同一时刻快照；不代表服务器完整历史。",
                "只导出文字及基础消息信息。媒体写入数量：0；原始XML、下载凭据及媒体内容不导出。",
            ],
        );
    }
    let mut out = crate::directory_export::Output(Zeroizing::new(Vec::new()));
    match format {
        LiveExportFormat::Json => {
            serde_json::to_writer_pretty(&mut out,&serde_json::json!({"schema_version":1,"data_mode":"selected_live_message_sources","account_id":account,"selection":selection,"source_count":sources,"cross_page_snapshot":false,"media_written":0,"rows":rows})).map_err(|_|limit())?;
        }
        LiveExportFormat::Csv | LiveExportFormat::Txt => {
            out.write_all(b"\xef\xbb\xbf").map_err(|_| limit())?;
            for values in
                std::iter::once(LABELS.map(str::to_owned).to_vec()).chain(rows.iter().map(fields))
            {
                let values = Zeroizing::new(values);
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        out.write_all(if matches!(format, LiveExportFormat::Csv) {
                            b","
                        } else {
                            b"\t"
                        })
                        .map_err(|_| limit())?;
                    }
                    if matches!(format, LiveExportFormat::Csv) {
                        crate::directory_export::csv(&mut out, value, matches!(i, 1 | 2 | 5))
                            .map_err(|_| limit())?;
                    } else {
                        let escaped = Zeroizing::new(
                            value
                                .replace('\\', "\\\\")
                                .replace('\t', "\\t")
                                .replace('\n', "\\n")
                                .replace('\r', "\\r"),
                        );
                        out.write_all(escaped.as_bytes()).map_err(|_| limit())?;
                    }
                }
                out.write_all(b"\r\n").map_err(|_| limit())?;
            }
        }
        LiveExportFormat::Html | LiveExportFormat::Md | LiveExportFormat::Xlsx => {
            unreachable!("handled above")
        }
    }
    Ok(out.0)
}
