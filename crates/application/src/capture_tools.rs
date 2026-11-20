//! Reports and exports read saved pages, without reopening any live source.
use crate::{Application, source_capture::CaptureRow};
use chrono::{Datelike, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
};
use werc_domain::{AppError, AppResult};
#[path = "capture_interchange.rs"]
pub(crate) mod interchange;
pub use interchange::capture_export_extension;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureFilter {
    #[serde(default)]
    pub message_kinds: Vec<String>,
    #[serde(default)]
    pub keyword: String,
    #[serde(default)]
    pub exclude_conversations: Vec<String>,
    #[serde(default)]
    pub view: AnalysisView,
    #[serde(default)]
    pub conversations: Vec<String>,
    pub from: Option<i64>,
    pub until: Option<i64>,
    #[serde(default)]
    pub deduplicate: bool,
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisView {
    #[default]
    All,
    Private,
    Group,
    Publisher,
}
impl CaptureFilter {
    pub(crate) fn valid(&self) -> bool {
        self.message_kinds.len() <= MESSAGE_KINDS.len()
            && self
                .message_kinds
                .iter()
                .all(|s| MESSAGE_KINDS.contains(&s.as_str()))
            && self.keyword.len() <= 1024
            && !self.keyword.contains('\0')
            && self.conversations.len() <= 10000
            && self.exclude_conversations.len() <= 10000
            && self
                .exclude_conversations
                .iter()
                .all(|s| werc_domain::live::valid_live_username(s))
            && self
                .conversations
                .iter()
                .all(|s| werc_domain::live::valid_live_username(s))
            && !matches!((self.from,self.until),(Some(a),Some(b)) if a>=b)
    }
    fn matches(&self, row: &CaptureRow) -> bool {
        (self.conversations.is_empty() || self.conversations.contains(&row.username))
            && !self.exclude_conversations.contains(&row.username)
            && match self.view {
                AnalysisView::All => true,
                AnalysisView::Private => werc_domain::live::is_private_account(&row.username),
                AnalysisView::Group => werc_domain::live::is_group_account(&row.username),
                AnalysisView::Publisher => row.username.starts_with("gh_"),
            }
            && (self.message_kinds.is_empty()
                || self
                    .message_kinds
                    .iter()
                    .any(|kind| kind == capture_kind(row)))
            && (self.keyword.is_empty()
                || row
                    .message
                    .text
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&self.keyword.to_lowercase()))
            && self.from.is_none_or(|v| row.message.timestamp >= v)
            && self.until.is_none_or(|v| row.message.timestamp < v)
    }
}
pub const MESSAGE_KINDS: &[&str] = &[
    "text",
    "image",
    "voice",
    "video",
    "emoticon",
    "file",
    "link",
    "forward",
    "quote",
    "transfer",
    "red_packet",
    "call",
    "location",
    "contact",
    "share",
    "system",
    "other",
];
pub(crate) fn capture_kind(row: &CaptureRow) -> &'static str {
    let raw = row.message.message_type.parse::<u64>().unwrap_or(u64::MAX);
    match raw & 0xffffffff {
        1 => "text",
        3 => "image",
        34 => "voice",
        43 | 62 => "video",
        47 => "emoticon",
        48 => "location",
        42 => "contact",
        50 => "call",
        10000 | 10002 => "system",
        49 => match row
            .app_subtype
            .or_else(|| u32::try_from(raw >> 32).ok().filter(|v| *v != 0))
        {
            Some(6) => "file",
            Some(5) => "link",
            Some(19) => "forward",
            Some(57) => "quote",
            Some(2000) => "transfer",
            Some(2001) => "red_packet",
            Some(3 | 4 | 33 | 36) => "share",
            _ => "other",
        },
        _ => "other",
    }
}
#[derive(Serialize)]
pub struct CaptureReport {
    pub generated_at: String,
    pub filter: CaptureFilter,
    pub self_username: String,
    pub sent: u64,
    pub received: u64,
    pub unknown_sender: u64,
    pub active_days: usize,
    pub longest_streak: usize,
    pub night_messages: u64,
    pub conversations: BTreeMap<String, ConversationStats>,
    pub common_phrases: Vec<(String, u64)>,
    pub phrase_comparison: PhraseComparison,
    pub monthly_leaders: BTreeMap<String, Vec<MonthlyLeader>>,
    pub capture_id: String,
    pub complete_selected_sources: bool,
    pub source_rows: u64,
    pub counted_rows: u64,
    pub duplicates_skipped: u64,
    pub readable_text_rows: u64,
    pub text_characters: u64,
    pub first_timestamp: Option<i64>,
    pub last_timestamp: Option<i64>,
    pub by_conversation: BTreeMap<String, u64>,
    pub by_sender: BTreeMap<String, u64>,
    pub by_type: BTreeMap<String, u64>,
    pub by_day: BTreeMap<String, u64>,
    pub by_month: BTreeMap<String, u64>,
    pub by_hour: [u64; 24],
    pub by_weekday: [u64; 7],
    pub timezone: String,
    pub historical_complete: bool,
}
#[derive(Default, Serialize)]
pub struct ConversationStats {
    pub total: u64,
    pub sent: u64,
    pub received: u64,
    pub unknown: u64,
    pub first: Option<i64>,
    pub last: Option<i64>,
    pub active_days: usize,
    pub longest_streak: usize,
    pub my_turns: u64,
    pub their_turns: u64,
    pub my_response_pairs: u64,
    pub their_response_pairs: u64,
    pub my_average_gap_seconds: Option<f64>,
    pub their_average_gap_seconds: Option<f64>,
    pub last_direction: String,
    pub by_sender: BTreeMap<String, u64>,
    pub by_type: BTreeMap<String, u64>,
    pub by_hour: [u64; 24],
    pub night_messages: u64,
    pub peak_day: Option<(String, u64)>,
    pub first_record: Option<ReportRecord>,
    pub last_record: Option<ReportRecord>,
}
#[derive(Serialize)]
pub struct ReportRecord {
    pub page: u64,
    pub row: usize,
    pub timestamp: i64,
    pub text: Option<String>,
    pub message_type: String,
    pub content_status: String,
}
#[derive(Default, Serialize)]
pub struct PhraseComparison {
    pub shared: Vec<(String, u64, u64)>,
    pub mine_only: Vec<(String, u64)>,
    pub others_only: Vec<(String, u64)>,
}
#[derive(Serialize)]
pub struct MonthlyLeader {
    pub username: String,
    pub messages: u64,
}
fn phrase_comparison(phrases: BTreeMap<String, (u64, u64)>) -> PhraseComparison {
    let mut result = PhraseComparison::default();
    for (text, (mine, others)) in phrases {
        if mine > 0 && others > 0 {
            result.shared.push((text, mine, others));
        } else if mine > 1 {
            result.mine_only.push((text, mine));
        } else if others > 1 {
            result.others_only.push((text, others));
        }
    }
    result
        .shared
        .sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)).then(a.0.cmp(&b.0)));
    result
        .mine_only
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    result
        .others_only
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    result.shared.truncate(100);
    result.mine_only.truncate(100);
    result.others_only.truncate(100);
    result
}
fn streak(days: &std::collections::BTreeSet<i32>) -> usize {
    let mut previous = None;
    let mut current = 0;
    let mut longest = 0;
    for &day in days {
        current = if previous == Some(day - 1) {
            current + 1
        } else {
            1
        };
        longest = longest.max(current);
        previous = Some(day);
    }
    longest
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureExportOptions {
    #[serde(default = "default_display_name")]
    pub display_name: String,
    #[serde(default)]
    pub media_kinds: Vec<String>,
    #[serde(default)]
    pub include_transcripts: bool,
    #[serde(default)]
    pub include_avatars: bool,
    #[serde(default)]
    pub xlsx_columns: Vec<String>,
    #[serde(default)]
    pub filter: CaptureFilter,
    #[serde(default)]
    pub redact_identifiers: bool,
    #[serde(default)]
    pub omit_text: bool,
    #[serde(default)]
    pub replacements: Vec<(String, String)>,
    pub text_template: Option<String>,
}
fn default_display_name() -> String {
    "username".into()
}
#[derive(Serialize)]
pub struct CaptureExportSummary {
    pub rows: u64,
    pub complete_selected_sources: bool,
    pub redacted: bool,
    pub media_written: u64,
}
fn failed() -> AppError {
    AppError::new("EXPORT_WRITE_FAILED", "导出写入失败；来源没有修改")
}
impl CaptureReport {
    pub fn export(&self, format: &str) -> AppResult<Vec<u8>> {
        if format == "json" {
            return serde_json::to_vec_pretty(self).map_err(|_| AppError::internal());
        }
        if !matches!(format, "txt" | "html") {
            return Err(AppError::new("INVALID_ARGUMENT", "报告支持TXT、HTML和JSON"));
        }
        let mut text = format!(
            "WeRC 聊天资料报告\n生成时间：{}\n来源：{}\n所选来源扫描{}；不代表服务器完整历史。\n\n消息 {} 条，发出 {}，收到 {}，身份未知 {}\n活跃 {} 天，最长连续 {} 天，深夜消息 {} 条\n可读文字 {} 条，{} 字符；去重 {} 条\n时区：系统本地时区（按每条消息时间）\n",
            self.generated_at,
            self.capture_id,
            if self.complete_selected_sources {
                "完成"
            } else {
                "未完成"
            },
            self.counted_rows,
            self.sent,
            self.received,
            self.unknown_sender,
            self.active_days,
            self.longest_streak,
            self.night_messages,
            self.readable_text_rows,
            self.text_characters,
            self.duplicates_skipped
        );
        text.push_str("\n会话排行\n");
        let mut rows: Vec<_> = self.conversations.iter().collect();
        rows.sort_by(|a, b| b.1.total.cmp(&a.1.total).then(a.0.cmp(b.0)));
        for (name, s) in rows {
            text.push_str(&format!(
                "{name}：{}条；我{} / 对方{} / 未知{}；活跃{}天；最长连续{}天\n",
                s.total, s.sent, s.received, s.unknown, s.active_days, s.longest_streak
            ));
            for (label, record) in [
                ("首次保存记录", &s.first_record),
                ("最近保存记录", &s.last_record),
            ] {
                if let Some(r) = record {
                    text.push_str(&format!(
                        "  {label}：{} {} {}（归档页{}行{}）\n",
                        r.timestamp,
                        r.text.as_deref().unwrap_or("[非文字消息]"),
                        local_marker(&r.content_status),
                        r.page + 1,
                        r.row + 1
                    ));
                }
            }
            if werc_domain::live::is_private_account(name) {
                text.push_str(&format!("  间隔后先发言：我{} / 对方{}。交替发言间隔：我{}秒（{}对样本）/ 对方{}秒（{}对样本）\n",s.my_turns,s.their_turns,s.my_average_gap_seconds.map(|v|format!("{v:.1}")).unwrap_or_else(||"未知".into()),s.my_response_pairs,s.their_average_gap_seconds.map(|v|format!("{v:.1}")).unwrap_or_else(||"未知".into()),s.their_response_pairs));
            }
            if let Some((day, count)) = &s.peak_day {
                text.push_str(&format!(
                    "  最忙的一天：{day}（{count}条）；深夜消息{}条\n",
                    s.night_messages
                ));
            }
            let mut senders: Vec<_> = s.by_sender.iter().collect();
            senders.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (sender, count) in senders.into_iter().take(100) {
                text.push_str(&format!("  发言：{sender} {count}条\n"));
            }
        }
        text.push_str("\n每月主要联系人（每月最多5位；不含群及公众号）\n");
        for (month, leaders) in &self.monthly_leaders {
            for leader in leaders {
                text.push_str(&format!(
                    "{month}：{} {}条\n",
                    leader.username, leader.messages
                ));
            }
        }
        text.push_str("\n双方共同短句（我的次数 / 对方次数）\n");
        for (phrase, mine, others) in &self.phrase_comparison.shared {
            text.push_str(&format!("{phrase}：{mine} / {others}\n"));
        }
        text.push_str("\n我的专属短句\n");
        for (phrase, count) in &self.phrase_comparison.mine_only {
            text.push_str(&format!("{phrase}：{count}\n"));
        }
        text.push_str("\n对方专属短句\n");
        for (phrase, count) in &self.phrase_comparison.others_only {
            text.push_str(&format!("{phrase}：{count}\n"));
        }
        text.push_str("\n月份\n");
        for (key, count) in &self.by_month {
            text.push_str(&format!("{key}：{count}\n"));
        }
        text.push_str("\n每日分布\n");
        for (key, count) in &self.by_day {
            text.push_str(&format!("{key}：{count}\n"));
        }
        text.push_str("\n小时分布\n");
        for (hour, count) in self.by_hour.iter().enumerate() {
            text.push_str(&format!("{hour:02}时：{count}\n"));
        }
        text.push_str("\n重复短句\n");
        for (phrase, count) in &self.common_phrases {
            text.push_str(&format!("{phrase}：{count}次\n"));
        }
        text.push_str("\n计算说明：6小时无消息后计作新一轮交流起点。交替发言间隔仅统计身份明确且相邻间隔1秒至24小时的私聊；同秒双方发言不推断顺序。该时间不代表已读或必然回复。短句为完全相同的2—40字符单行文本。资料不足时结果只代表已保存部分。\n");
        if format == "txt" {
            return Ok(text.into_bytes());
        }
        Ok(format!("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><title>WeRC 聊天资料报告</title><style>body{{max-width:960px;margin:3rem auto;padding:0 1rem;font:16px/1.7 system-ui,sans-serif;color:#242424}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit}}@media print{{body{{margin:0}}}}</style></head><body><pre>{}</pre></body></html>",html(&text)).into_bytes())
    }
}
fn duplicate_key(row: &CaptureRow) -> String {
    match row.message.server_id.as_deref().filter(|v| *v != "0") {
        Some(id) => format!("{}:server:{id}", row.username),
        None => format!(
            "{}:source:{}:local:{}",
            row.username, row.source_index, row.message.local_id
        ),
    }
}
fn html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn csv(s: &str) -> String {
    let formula = s.trim_start().starts_with(['=', '+', '-', '@']);
    format!(
        "\"{}{}\"",
        if formula { "'" } else { "" },
        s.replace('"', "\"\"")
    )
}
impl Application {
    pub(crate) fn capture_revision_snapshot(
        &self,
        account: &str,
        id: &str,
        check: impl Fn() -> AppResult<()>,
    ) -> AppResult<String> {
        check()?;
        let root = self.repository.library_get(account, id)?.header;
        if root.kind != "source_capture" {
            return Err(AppError::new("DOCUMENT_CORRUPTED", "归档类型无效"));
        }
        let status = self.source_capture_status_local(account, id)?;
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&(account, id, root.revision, status.pages))
            .map_err(|_| AppError::internal())?);
        for index in 0..status.pages {
            check()?;
            let page = self.repository.library_get(account, &format!("{id}:{index}"))?.header;
            if page.kind != "capture_page" {
                return Err(AppError::new("DOCUMENT_CORRUPTED", "归档页类型无效"));
            }
            digest.update(page.revision.to_le_bytes());
        }
        let current = self.repository.library_get(account, id)?.header;
        if current.revision != root.revision || current.kind != root.kind {
            return Err(AppError::new("DOCUMENT_CONFLICT", "读取期间归档已变化，请重新读取"));
        }
        check()?;
        Ok(hex::encode(digest.finalize()))
    }
    pub(crate) fn visit_capture_local(
        &self,
        account: &str,
        id: &str,
        filter: &CaptureFilter,
        request_id: &str,
        mut visit: impl FnMut(CaptureRow) -> AppResult<()>,
    ) -> AppResult<(bool, u64, u64)> {
        self.visit_capture_positions_local(account, id, filter, request_id, |_, _, row| visit(row))
    }
    pub(crate) fn visit_capture_positions_local(
        &self,
        account: &str,
        id: &str,
        filter: &CaptureFilter,
        request_id: &str,
        visit: impl FnMut(u64, usize, CaptureRow) -> AppResult<()>,
    ) -> AppResult<(bool, u64, u64)> {
        let lease = self.browse_lease(request_id)?.for_local_archive();
        self.visit_capture_positions_checked(account, id, filter, || lease.check(), visit)
    }
    pub(crate) fn visit_capture_positions_checked(
        &self,
        account: &str,
        id: &str,
        filter: &CaptureFilter,
        check: impl Fn() -> AppResult<()>,
        mut visit: impl FnMut(u64, usize, CaptureRow) -> AppResult<()>,
    ) -> AppResult<(bool, u64, u64)> {
        if !filter.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "会话或日期筛选无效"));
        }
        let root = self.repository.library_get(account, id)?.header;
        let status = self.source_capture_status_local(account, id)?;
        let mut seen = HashSet::new();
        let mut duplicates = 0;
        let mut revisions = Vec::new();
        for index in 0..status.pages {
            check()?;
            let page = self.source_capture_page_local(account, id, index)?;
            revisions.push(page.revision);
            for (position, row) in page.rows.into_iter().enumerate() {
                if row.message.content_status == "deleted_local" || !filter.matches(&row) {
                    continue;
                }
                if filter.deduplicate && !seen.insert(duplicate_key(&row)) {
                    duplicates += 1;
                    continue;
                }
                visit(index, position, row)?;
            }
        }
        for (index, revision) in revisions.into_iter().enumerate() {
            check()?;
            let current = self
                .repository
                .library_get(account, &format!("{id}:{index}"))?;
            if current.header.kind != "capture_page" || current.header.revision != revision {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "导出或分析期间资料被修改，请重新读取",
                ));
            }
        }
        check()?;
        let current = self.repository.library_get(account, id)?.header;
        if current.revision != root.revision || current.kind != root.kind {
            return Err(AppError::new(
                "DOCUMENT_CONFLICT",
                "导出或分析期间归档范围已变化，请重新读取",
            ));
        }
        Ok((status.complete_selected_sources, status.rows, duplicates))
    }
    pub fn capture_report_local(
        &self,
        account: &str,
        id: &str,
        filter: &CaptureFilter,
        request_id: &str,
    ) -> AppResult<CaptureReport> {
        let login = self.capture_login_local(account, id)?;
        let mut days = std::collections::BTreeSet::new();
        let mut conversation_days = BTreeMap::<String, std::collections::BTreeSet<i32>>::new();
        let mut timeline = BTreeMap::<String, Vec<(i64, i8)>>::new();
        let mut phrases = BTreeMap::<String, u64>::new();
        let mut directional_phrases = BTreeMap::<String, (u64, u64)>::new();
        let mut monthly_contacts = BTreeMap::<String, BTreeMap<String, u64>>::new();
        let mut conversation_daily = BTreeMap::<String, BTreeMap<String, u64>>::new();
        let mut report = CaptureReport {
            generated_at: chrono::Utc::now().to_rfc3339(),
            filter: filter.clone(),
            self_username: login.clone(),
            sent: 0,
            received: 0,
            unknown_sender: 0,
            active_days: 0,
            longest_streak: 0,
            night_messages: 0,
            conversations: BTreeMap::new(),
            common_phrases: vec![],
            phrase_comparison: PhraseComparison::default(),
            monthly_leaders: BTreeMap::new(),
            capture_id: id.into(),
            complete_selected_sources: false,
            source_rows: 0,
            counted_rows: 0,
            duplicates_skipped: 0,
            readable_text_rows: 0,
            text_characters: 0,
            first_timestamp: None,
            last_timestamp: None,
            by_conversation: BTreeMap::new(),
            by_sender: BTreeMap::new(),
            by_type: BTreeMap::new(),
            by_day: BTreeMap::new(),
            by_month: BTreeMap::new(),
            by_hour: [0; 24],
            by_weekday: [0; 7],
            timezone: "system_local_per_timestamp".into(),
            historical_complete: false,
        };
        let (complete, rows, duplicates) = self.visit_capture_positions_local(
            account,
            id,
            filter,
            request_id,
            |page, position, row| {
                report.counted_rows += 1;
                let m = &row.message;
                let stats = report
                    .conversations
                    .entry(row.username.clone())
                    .or_default();
                stats.total += 1;
                stats.first = Some(stats.first.map_or(m.timestamp, |n| n.min(m.timestamp)));
                stats.last = Some(stats.last.map_or(m.timestamp, |n| n.max(m.timestamp)));
                let observation = || ReportRecord {
                    page,
                    row: position,
                    timestamp: m.timestamp,
                    text: if m.message_type == "1" {
                        m.text.as_ref().map(|s| s.chars().take(160).collect())
                    } else {
                        None
                    },
                    message_type: m.message_type.clone(),
                    content_status: m.content_status.clone(),
                };
                let key = (m.timestamp, page, position);
                if stats
                    .first_record
                    .as_ref()
                    .is_none_or(|r| key < (r.timestamp, r.page, r.row))
                {
                    stats.first_record = Some(observation());
                }
                if stats
                    .last_record
                    .as_ref()
                    .is_none_or(|r| key > (r.timestamp, r.page, r.row))
                {
                    stats.last_record = Some(observation());
                }
                let direction = match m.sender_username.as_deref() {
                    Some(sender) if !login.is_empty() && sender == login => {
                        report.sent += 1;
                        stats.sent += 1;
                        1
                    }
                    Some(sender)
                        if !login.is_empty()
                            && (werc_domain::live::is_group_account(&row.username)
                                || sender == row.username) =>
                    {
                        report.received += 1;
                        stats.received += 1;
                        -1
                    }
                    _ => {
                        report.unknown_sender += 1;
                        stats.unknown += 1;
                        0
                    }
                };
                if let Some(sender) = m.sender_username.as_ref().filter(|_| direction != 0) {
                    *stats.by_sender.entry(sender.clone()).or_default() += 1;
                }
                *stats.by_type.entry(m.message_type.clone()).or_default() += 1;
                if werc_domain::live::is_private_account(&row.username) {
                    timeline
                        .entry(row.username.clone())
                        .or_default()
                        .push((m.timestamp, direction));
                }
                *report
                    .by_conversation
                    .entry(row.username.clone())
                    .or_default() += 1;
                *report
                    .by_sender
                    .entry(
                        m.sender_username
                            .clone()
                            .unwrap_or_else(|| "[未知发送者]".into()),
                    )
                    .or_default() += 1;
                *report.by_type.entry(m.message_type.clone()).or_default() += 1;
                if let Some(text) = &m.text {
                    report.readable_text_rows += 1;
                    report.text_characters += text.chars().count() as u64;
                    let phrase = text.trim();
                    if (2..=40).contains(&phrase.chars().count()) && !phrase.contains(['\n', '\r'])
                    {
                        *phrases.entry(phrase.to_owned()).or_default() += 1;
                        // Only ordinary text with a known direction contributes to
                        // the comparison. XML/card titles are not spoken phrases.
                        if m.message_type == "1" && direction != 0 {
                            let counts = directional_phrases.entry(phrase.into()).or_default();
                            if direction == 1 {
                                counts.0 += 1;
                            } else {
                                counts.1 += 1;
                            }
                        }
                    }
                }
                report.first_timestamp = Some(
                    report
                        .first_timestamp
                        .map_or(m.timestamp, |v| v.min(m.timestamp)),
                );
                report.last_timestamp = Some(
                    report
                        .last_timestamp
                        .map_or(m.timestamp, |v| v.max(m.timestamp)),
                );
                if let Some(date) = chrono::Local.timestamp_opt(m.timestamp, 0).single() {
                    let day = date.date_naive().num_days_from_ce();
                    days.insert(day);
                    conversation_days
                        .entry(row.username.clone())
                        .or_default()
                        .insert(day);
                    if date.hour() < 6 {
                        report.night_messages += 1;
                        stats.night_messages += 1;
                    }
                    stats.by_hour[date.hour() as usize] += 1;
                    let date_key = date.format("%Y-%m-%d").to_string();
                    *conversation_daily
                        .entry(row.username.clone())
                        .or_default()
                        .entry(date_key)
                        .or_default() += 1;
                    if werc_domain::live::is_private_account(&row.username) {
                        *monthly_contacts
                            .entry(date.format("%Y-%m").to_string())
                            .or_default()
                            .entry(row.username.clone())
                            .or_default() += 1;
                    }
                    *report
                        .by_day
                        .entry(date.format("%Y-%m-%d").to_string())
                        .or_default() += 1;
                    *report
                        .by_month
                        .entry(date.format("%Y-%m").to_string())
                        .or_default() += 1;
                    report.by_hour[date.hour() as usize] += 1;
                    report.by_weekday[date.weekday().num_days_from_monday() as usize] += 1;
                }
                Ok(())
            },
        )?;
        report.complete_selected_sources = complete;
        report.source_rows = rows;
        report.duplicates_skipped = duplicates;
        report.active_days = days.len();
        report.longest_streak = streak(&days);
        report.phrase_comparison = phrase_comparison(directional_phrases);
        for (month, counts) in monthly_contacts {
            let mut sorted: Vec<_> = counts.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            sorted.truncate(5);
            report.monthly_leaders.insert(
                month,
                sorted
                    .into_iter()
                    .map(|(username, messages)| MonthlyLeader { username, messages })
                    .collect(),
            );
        }
        for (username, counts) in conversation_daily {
            if let Some(stats) = report.conversations.get_mut(&username) {
                stats.peak_day = counts
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
            }
        }
        for (name, days) in conversation_days {
            if let Some(stats) = report.conversations.get_mut(&name) {
                stats.active_days = days.len();
                stats.longest_streak = streak(&days);
            }
        }
        for (name, mut rows) in timeline {
            rows.sort_unstable_by_key(|row| row.0);
            let stats = report
                .conversations
                .get_mut(&name)
                .ok_or_else(AppError::internal)?;
            let mut my_gap = 0u64;
            let mut their_gap = 0u64;
            // Same-second ordering is not evidence of who answered whom. Mark the whole second unknown.
            let mut collapsed = Vec::<(i64, i8)>::new();
            for (time, direction) in rows {
                if let Some(last) = collapsed.last_mut().filter(|r| r.0 == time) {
                    if last.1 != direction {
                        last.1 = 0;
                    }
                } else {
                    collapsed.push((time, direction));
                }
            }
            let mut previous: Option<(i64, i8)> = None;
            for (time, direction) in collapsed {
                if direction == 0 {
                    previous = None;
                    stats.last_direction = "unknown".into();
                    continue;
                }
                stats.last_direction = if direction == 1 { "sent" } else { "received" }.into();
                let new_turn = previous.is_none_or(|(old, _)| time - old > 6 * 3600);
                if new_turn {
                    if direction == 1 {
                        stats.my_turns += 1;
                    } else {
                        stats.their_turns += 1;
                    }
                }
                if let Some((old, last)) = previous {
                    let gap = time - old;
                    if last != direction && (1..=86400).contains(&gap) {
                        if direction == 1 {
                            stats.my_response_pairs += 1;
                            my_gap += gap as u64;
                        } else {
                            stats.their_response_pairs += 1;
                            their_gap += gap as u64;
                        }
                    }
                }
                previous = Some((time, direction));
            }
            stats.my_average_gap_seconds = (stats.my_response_pairs > 0)
                .then(|| my_gap as f64 / stats.my_response_pairs as f64);
            stats.their_average_gap_seconds = (stats.their_response_pairs > 0)
                .then(|| their_gap as f64 / stats.their_response_pairs as f64);
        }
        let mut phrases: Vec<_> = phrases.into_iter().filter(|(_, n)| *n > 1).collect();
        phrases.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        phrases.truncate(100);
        report.common_phrases = phrases;
        Ok(report)
    }
    pub fn export_capture_local(
        &self,
        account: &str,
        id: &str,
        format: &str,
        options: &CaptureExportOptions,
        request_id: &str,
        out: &mut dyn Write,
    ) -> AppResult<CaptureExportSummary> {
        if capture_export_extension(format).is_none()
            || options.replacements.len() > 64
            || options
                .replacements
                .iter()
                .any(|(a, b)| a.is_empty() || a.len() > 1024 || b.len() > 1024)
            || options
                .text_template
                .as_ref()
                .is_some_and(|s| s.len() > 16384)
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "导出格式、脱敏替换或模板无效",
            ));
        }
        interchange::validate_columns(&options.xlsx_columns)?;
        if !matches!(
            options.display_name.as_str(),
            "" | "username" | "remark" | "nickname" | "group_nickname"
        ) || options.media_kinds.len() > 5
            || options.media_kinds.iter().any(|k| {
                !matches!(
                    k.as_str(),
                    "image" | "voice" | "video" | "emoticon" | "file"
                )
            })
        {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "显示名偏好或媒体类型无效",
            ));
        }
        if matches!(
            format,
            "xlsx" | "sql" | "chatlab_json" | "chatlab_jsonl" | "chatlab_zip" | "weclone_csv"
        ) {
            return self.export_capture_interchange(account, id, format, options, request_id, out);
        }
        let mut salt = [0u8; 32];
        getrandom::fill(&mut salt).map_err(|_| AppError::internal())?;
        let alias = |value: &str| {
            let mut h = Sha256::new();
            h.update(salt);
            h.update(value.as_bytes());
            format!("匿名-{}", &hex::encode(h.finalize())[..12])
        };
        let names = self.capture_names_local(account, id)?;
        let mut rows = 0;
        match format {
            "json"=>out.write_all(b"{\"schema_version\":1,\"data_mode\":\"saved_source_capture\",\"rows\":[").map_err(|_|failed())?,
            "csv"=>out.write_all(b"\xef\xbb\xbfconversation,source,local_id,server_id,time,sender,type,text,local_status\r\n").map_err(|_|failed())?,
            "html"=>out.write_all(b"<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none';base-uri 'none';form-action 'none'\"><title>WeRC archive</title></head><body><h1>WeRC archive</h1>").map_err(|_|failed())?,
            _=>{},
        }
        let (complete, _, _) =
            self.visit_capture_local(account, id, &options.filter, request_id, |mut row| {
                if options.redact_identifiers {
                    row.username = alias(&row.username);
                    row.message.sender_username = row.message.sender_username.as_deref().map(alias);
                    row.message.server_id = None;
                    row.message.local_id = (rows + 1).to_string();
                }
                if options.omit_text {
                    row.message.text = None;
                } else if let Some(text) = &mut row.message.text {
                    for (a, b) in &options.replacements {
                        *text = text.replace(a, b);
                    }
                }
                let m = &row.message;
                let sender = names.display(
                    &row.username,
                    m.sender_username.as_deref().unwrap_or(""),
                    &options.display_name,
                );
                let conversation =
                    names.display(&row.username, &row.username, &options.display_name);
                let date = chrono::DateTime::from_timestamp(m.timestamp, 0)
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_default();
                match format {
                    "json" | "jsonl" => {
                        if format == "json" && rows > 0 {
                            out.write_all(b",").map_err(|_| failed())?;
                        }
                        serde_json::to_writer(&mut *out, &row).map_err(|_| failed())?;
                        if format == "jsonl" {
                            out.write_all(b"\n").map_err(|_| failed())?;
                        }
                    }
                    "csv" => {
                        let values = [
                            row.username.clone(),
                            row.source_index.to_string(),
                            m.local_id.clone(),
                            m.server_id.clone().unwrap_or_default(),
                            date,
                            m.sender_username.clone().unwrap_or_default(),
                            m.message_type.clone(),
                            m.text.clone().unwrap_or_default(),
                            m.content_status.clone(),
                        ];
                        writeln!(
                            out,
                            "{}\r",
                            values.iter().map(|s| csv(s)).collect::<Vec<_>>().join(",")
                        )
                        .map_err(|_| failed())?;
                    }
                    "html" => {
                        write!(
                            out,
                            "<article><h2>{}</h2><p>{} · {} · {}</p><pre>{}</pre></article>",
                            html(&conversation),
                            html(&date),
                            html(&sender),
                            local_marker(&m.content_status),
                            html(m.text.as_deref().unwrap_or("[非文字或未读取正文]"))
                        )
                        .map_err(|_| failed())?;
                    }
                    _ => {
                        let template = options
                            .text_template
                            .as_deref()
                            .unwrap_or("[{{date}}] {{conversation}} / {{sender}}\n{{text}}\n");
                        // Parse placeholders once; message contents cannot introduce new placeholders.
                        let mut remaining = template;
                        let mut rendered = if local_marker(&m.content_status).is_empty() {
                            String::new()
                        } else {
                            format!("[{}]\n", local_marker(&m.content_status))
                        };
                        while let Some(start) = remaining.find("{{") {
                            rendered.push_str(&remaining[..start]);
                            let Some(end) = remaining[start + 2..].find("}}") else {
                                rendered.push_str(&remaining[start..]);
                                remaining = "";
                                break;
                            };
                            let token = &remaining[start + 2..start + 2 + end];
                            let replacement = match token {
                                "date" => date.as_str(),
                                "conversation" => conversation.as_str(),
                                "sender" => sender.as_str(),
                                "type" => m.message_type.as_str(),
                                "text" => m.text.as_deref().unwrap_or("[非文字或未读取正文]"),
                                _ => "",
                            };
                            rendered.push_str(replacement);
                            remaining = &remaining[start + end + 4..];
                        }
                        rendered.push_str(remaining);
                        if format == "md" {
                            for line in rendered.replace('\r', "").split('\n') {
                                writeln!(out, "    {line}").map_err(|_| failed())?;
                            }
                        } else {
                            out.write_all(rendered.as_bytes()).map_err(|_| failed())?;
                        }
                        out.write_all(b"\n").map_err(|_| failed())?;
                    }
                }
                rows += 1;
                Ok(())
            })?;
        if format == "json" {
            write!(out,"],\"complete_selected_sources\":{complete},\"historical_complete\":false,\"media_written\":0}}").map_err(|_|failed())?;
        }
        if format == "html" {
            out.write_all(b"</body></html>").map_err(|_| failed())?;
        }
        Ok(CaptureExportSummary {
            rows,
            complete_selected_sources: complete,
            redacted: options.redact_identifiers
                || options.omit_text
                || !options.replacements.is_empty(),
            media_written: 0,
        })
    }
}

pub(crate) fn local_marker(status: &str) -> &str {
    match status {
        "edited_local" => "本地已修改",
        "added_local" => "本地新增记录",
        "deleted_local" => "本地已删除",
        _ => "",
    }
}

#[cfg(test)]
mod account_filter_tests {
    use super::*;
    #[test]
    fn private_group_and_publisher_exports_do_not_overlap() {
        for (username, expected) in [
            ("owned", AnalysisView::Private),
            ("owned@chatroom", AnalysisView::Group),
            ("owned@im.chatroom", AnalysisView::Group),
            ("gh_owned", AnalysisView::Publisher),
        ] {
            let row:CaptureRow=serde_json::from_value(serde_json::json!({"username":username,"source_index":0,"message":{"local_id":"1","timestamp":1700000000,"message_type":"1","text":"自动化测试","content_status":"valid"}})).unwrap();
            for view in [
                AnalysisView::Private,
                AnalysisView::Group,
                AnalysisView::Publisher,
            ] {
                let filter = CaptureFilter {
                    view: view.clone(),
                    ..Default::default()
                };
                assert_eq!(filter.matches(&row), view == expected, "{username}");
            }
        }
    }
}
