use super::*;
use serde::Deserialize;
use std::{collections::BTreeMap, io::Write};
use werc_domain::live::{LiveCalendarDay, LiveCalendarSelection};

#[derive(Serialize)]
pub struct MomentOverview {
    pub days: Vec<LiveCalendarDay>,
    pub counts: Vec<u64>,
    pub authors: BTreeMap<String, u64>,
    pub scanned: usize,
    pub unreadable: usize,
    pub complete_timeline: bool,
    pub cross_page_snapshot: bool,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MomentExportFormat {
    Json,
    Html,
    Md,
    Txt,
}
impl MomentExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Html => "html",
            Self::Md => "md",
            Self::Txt => "txt",
        }
    }
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Html => "text/html; charset=utf-8",
            Self::Md => "text/markdown; charset=utf-8",
            Self::Txt => "text/plain; charset=utf-8",
        }
    }
}
#[derive(Serialize)]
pub struct MomentExportSummary {
    pub rows: usize,
    pub scanned: usize,
    pub bytes: usize,
    pub missing_media: u64,
    pub unreadable: usize,
    pub complete_timeline: bool,
    pub cross_page_snapshot: bool,
}
pub struct MomentExport {
    pub summary: MomentExportSummary,
    pub bytes: zeroize::Zeroizing<Vec<u8>>,
}
fn failed() -> AppError {
    AppError::new("EXPORT_FAILED", "朋友圈导出失败")
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
impl Application {
    pub fn moment_overview_local(
        &self,
        account: &str,
        view: &str,
        query: &MomentQuery,
        days: &[LiveCalendarDay],
        request: &str,
    ) -> AppResult<MomentOverview> {
        let selection = LiveCalendarSelection {
            username: "filehelper".into(),
            days: days.to_vec(),
        };
        if !selection.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "日历需要连续的最多31天"));
        }
        let lease = self.source_lease(request)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        let mut result = MomentOverview {
            days: days.to_vec(),
            counts: vec![0; days.len()],
            authors: BTreeMap::new(),
            scanned: 0,
            unreadable: 0,
            complete_timeline: false,
            cross_page_snapshot: false,
        };
        result.scanned = self.scan_moments(account, view, query, &lease, |row| {
            *result
                .authors
                .entry(row.anchor.username.clone())
                .or_default() += 1;
            if let Some(content) = &row.content {
                if let Some(i) = days.iter().position(|d| {
                    content.timestamp >= d.from_timestamp && content.timestamp < d.until_timestamp
                }) {
                    result.counts[i] += 1;
                }
            } else {
                result.unreadable += 1;
            }
            Ok(())
        })?;
        Ok(result)
    }
    pub fn export_live_moments_local<T>(
        &self,
        account: &str,
        view: &str,
        query: &MomentQuery,
        format: MomentExportFormat,
        request: &str,
        publish: impl FnOnce(MomentExport) -> AppResult<T>,
    ) -> AppResult<T> {
        let lease = self.source_lease(request)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        match format {
            MomentExportFormat::Json=>bytes.extend_from_slice(b"{\"mode\":\"local_moment_cache\",\"complete_timeline\":false,\"cross_page_snapshot\":false,\"rows\":["),
            MomentExportFormat::Html=>bytes.extend_from_slice("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'\"><title>WeRC 朋友圈</title><style>body{font:16px system-ui;max-width:850px;margin:auto;padding:24px}article{border-bottom:1px solid #bbb;padding:16px 0}pre{white-space:pre-wrap;overflow-wrap:anywhere}small{color:#666}</style><h1>本地朋友圈</h1><p>来自当前电脑缓存，不代表完整时间线。媒体未包含在此文件中；赞和评论为读取时缓存数量。</p>".as_bytes()),
            _=>bytes.extend_from_slice("本地朋友圈\n来自当前电脑缓存，不代表完整时间线；媒体未包含，赞和评论为缓存数量。\n\n".as_bytes()),
        }
        let (mut count, mut unreadable, mut missing) = (0, 0, 0u64);
        let scanned=self.scan_moments(account,view,query,&lease,|row|{
            let (time,text,media,likes,comments)=if let Some(c)=&row.content{(c.timestamp.to_string(),c.text.as_str(),c.media_count,c.cached_like_count,c.cached_comment_count)}else{unreadable+=1;(String::new(),"内容无法读取",0,0,0)};
            missing+=u64::from(media);
            let display_details=row.content.as_ref().map(|c|details_text(&c.details)).unwrap_or_default();
            let display=format!("{text}\n{display_details}");let text=display.as_str();
            match format {
                MomentExportFormat::Json=>{if count>0{bytes.push(b',');}serde_json::to_writer(&mut *bytes,&row).map_err(|_|failed())?;},
                MomentExportFormat::Html=>write!(&mut *bytes,"<article><h2>{}</h2><small>时间戳 {} · 动态 {} · 状态 {}</small><pre>{}</pre><small>未包含媒体 {} · 缓存赞 {} · 缓存评论 {}</small></article>",escape(&row.anchor.username),time,escape(&row.anchor.id),escape(&row.content_status),escape(text),media,likes,comments).map_err(|_|failed())?,
                MomentExportFormat::Md=>{let content=format!("作者：{}\n时间戳：{time}\n动态：{}\n状态：{}\n{text}\n未包含媒体：{media} · 缓存赞：{likes} · 缓存评论：{comments}\n",row.anchor.username,row.anchor.id,row.content_status);for line in content.split('\n'){writeln!(&mut *bytes,"    {line}").map_err(|_|failed())?;}bytes.extend_from_slice(b"\n---\n\n");},
                MomentExportFormat::Txt=>writeln!(&mut *bytes,"作者：{}\n时间戳：{time}\n动态：{}\n状态：{}\n{text}\n未包含媒体：{media} · 缓存赞：{likes} · 缓存评论：{comments}\n",row.anchor.username,row.anchor.id,row.content_status).map_err(|_|failed())?,
            }
            count+=1;
            if bytes.len()>32*1024*1024{return Err(AppError::new("EXPORT_LIMIT","导出超过32MiB，请缩小范围"));}Ok(())
        })?;
        if matches!(format, MomentExportFormat::Json) {
            write!(
                &mut *bytes,
                "],\"count\":{count},\"missing_media\":{missing},\"unreadable\":{unreadable}}}"
            )
            .map_err(|_| failed())?;
        }
        if matches!(format, MomentExportFormat::Html) {
            bytes.extend_from_slice(b"</html>");
        }
        lease.check()?;
        publish(MomentExport {
            summary: MomentExportSummary {
                rows: count,
                scanned,
                bytes: bytes.len(),
                missing_media: missing,
                unreadable,
                complete_timeline: false,
                cross_page_snapshot: false,
            },
            bytes,
        })
    }
}

pub(crate) fn details_text(d: &MomentDetails) -> String {
    let mut text = String::new();
    for (label, value) in [
        ("标题", d.title.as_str()),
        ("摘要", d.description.as_str()),
        ("来源", d.source_name.as_str()),
        ("链接", d.link.as_deref().unwrap_or("")),
    ] {
        if !value.is_empty() {
            text.push_str(&format!("{label}：{value}\n"));
        }
    }
    for (k, v) in &d.location {
        text.push_str(&format!("位置 {k}：{v}\n"));
    }
    for m in &d.media {
        text.push_str(&format!(
            "媒体 {} · 类型 {:?} · {} · {} · {}×{} · 时长 {} · 实况 {}\n",
            m.slot + 1,
            m.kind,
            m.title,
            m.description,
            m.width.as_deref().unwrap_or("?"),
            m.height.as_deref().unwrap_or("?"),
            m.duration.as_deref().unwrap_or("?"),
            m.live_photo
        ));
    }
    for (kind, rows) in [("赞", &d.likes), ("评论", &d.comments)] {
        for r in rows {
            text.push_str(&format!("{kind} · {}（{}）· 时间 {:?} · 编号 {} / {} · 回复 {} / {}（{}）· 删除标记 {:?}\n{}\n",r.nickname,r.username,r.timestamp,r.id.as_deref().unwrap_or(""),r.id64.as_deref().unwrap_or(""),r.reply_id.as_deref().unwrap_or(""),r.reply_id64.as_deref().unwrap_or(""),r.reply_username,r.deleted,r.text));
        }
    }
    text
}
