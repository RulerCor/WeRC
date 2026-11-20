use crate::{Application, moment_media::MomentMediaManifest};
use serde::Serialize;
use std::io::{Seek, Write};
use werc_domain::{AppError, AppResult, moments::*};
use zeroize::Zeroizing;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};
fn failure() -> AppError {
    AppError::new("EXPORT_WRITE_FAILED", "朋友圈离线包写入失败")
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
#[derive(Default, Serialize)]
pub struct MomentBundleSummary {
    pub rows: u64,
    pub media: u64,
    pub missing_media: u64,
    pub complete_timeline: bool,
}
impl Application {
    pub fn export_moment_bundle_local<W: Write + Seek>(
        &self,
        account: &str,
        view: Option<&str>,
        query: &MomentQuery,
        request: &str,
        out: W,
    ) -> AppResult<MomentBundleSummary> {
        if !query.valid() {
            return Err(AppError::new("INVALID_ARGUMENT", "朋友圈筛选无效"));
        }
        let _guard = self
            .history_guard
            .try_lock()
            .map_err(|_| AppError::new("BUSY", "已有来源读取正在执行"))?;
        let lease = self.browse_lease(request)?.for_local_archive();
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(true);
        let mut zip = ZipWriter::new(out);
        let mut summary = MomentBundleSummary::default();
        let mut saved = std::collections::BTreeMap::new();
        let mut before = None;
        loop {
            lease.check()?;
            let page = self
                .repository
                .library_list(account, "moment_media", before, 100)?;
            for header in page.data {
                let (_, m) =
                    self.open_document::<MomentMediaManifest>(account, &header.id, "moment_media")?;
                let key = (
                    m.moment_id.clone(),
                    m.username.clone(),
                    m.slot,
                    m.variant as u8,
                );
                saved.entry(key).or_insert(m);
            }
            let Some(next) = page.next_cursor else { break };
            before = Some(next);
        }
        let mut html = Zeroizing::new(String::from(
            "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none';img-src 'self';media-src 'self';style-src 'unsafe-inline';base-uri 'none'\"><title>朋友圈离线副本</title><style>body{font:16px system-ui;max-width:900px;margin:auto;padding:24px}pre{white-space:pre-wrap;overflow-wrap:anywhere}img,video{max-width:100%;max-height:70vh}article{border-bottom:1px solid #ccc;padding:16px 0}</style><h1>朋友圈离线副本</h1><p>本机可取得的缓存及已保存媒体，不代表完整远端时间线。缺失资源逐项列在 manifest.json。</p>",
        ));
        let mut jsonl = Zeroizing::new(Vec::new());
        let mut outcomes = Vec::new();
        let mut consume = |row: LiveMoment| -> AppResult<()> {
            lease.check()?;
            summary.rows += 1;
            serde_json::to_writer(&mut *jsonl, &row).map_err(|_| failure())?;
            jsonl.push(b'\n');
            let text = row
                .content
                .as_ref()
                .map(|c| {
                    format!(
                        "{}\n{}",
                        c.text,
                        crate::live_moments::details_text(&c.details)
                    )
                })
                .unwrap_or_else(|| "内容不可读".into());
            html.push_str(&format!(
                "<article><h2>{}</h2><pre>{}</pre>",
                escape(&row.anchor.username),
                escape(&text)
            ));
            if let Some(content) = &row.content {
                for media in &content.details.media {
                    let variants = if media.live_photo {
                        vec![MomentMediaVariant::Original, MomentMediaVariant::LiveVideo]
                    } else {
                        vec![MomentMediaVariant::Original]
                    };
                    for variant in variants {
                        let key = (
                            row.anchor.id.clone(),
                            row.anchor.username.clone(),
                            media.slot,
                            variant as u8,
                        );
                        if let Some(m) = saved.get(&key) {
                            let (m, bytes) = self.moment_media_contents_local(account, &m.id)?;
                            let ext = if m.mime == "video/mp4" { "mp4" } else { "png" };
                            let path = format!(
                                "media/{}-{}-{}.{}",
                                summary.rows, media.slot, variant as u8, ext
                            );
                            zip.start_file(&path, options).map_err(|_| failure())?;
                            zip.write_all(&bytes).map_err(|_| failure())?;
                            if m.mime == "video/mp4" {
                                html.push_str(&format!(
                                    "<video controls preload=\"metadata\" src=\"{path}\"></video>"
                                ));
                            } else {
                                html.push_str(&format!(
                                    "<img loading=\"lazy\" src=\"{path}\" alt=\"第{}项图片\">",
                                    media.slot + 1
                                ));
                            }
                            summary.media += 1;
                            outcomes.push(serde_json::json!({"moment_id":row.anchor.id,"author":row.anchor.username,"slot":media.slot,"variant":variant,"status":"saved","path":path,"sha256":m.sha256,"size":m.size}));
                        } else {
                            summary.missing_media += 1;
                            html.push_str(&format!(
                                "<p>第{}项{}尚未保存</p>",
                                media.slot + 1,
                                if variant == MomentMediaVariant::LiveVideo {
                                    "动态片段"
                                } else {
                                    "媒体"
                                }
                            ));
                            outcomes.push(serde_json::json!({"moment_id":row.anchor.id,"author":row.anchor.username,"slot":media.slot,"variant":variant,"status":"not_saved"}));
                        }
                    }
                }
                let unparsed =
                    (content.media_count as usize).saturating_sub(content.details.media.len());
                summary.missing_media += unparsed as u64;
                if unparsed > 0 {
                    outcomes.push(serde_json::json!({"moment_id":row.anchor.id,"author":row.anchor.username,"status":"metadata_missing","count":unparsed}));
                }
            }
            html.push_str("</article>");
            if html.len() + jsonl.len() > 64 * 1024 * 1024 {
                return Err(AppError::new(
                    "EXPORT_SIZE_LIMIT",
                    "正文过多，请按作者或时间分批导出",
                ));
            }
            Ok(())
        };
        if let Some(view) = view {
            self.scan_moments(account, view, query, &lease, &mut consume)?;
        } else {
            let mut before = None;
            loop {
                let page = self.preserved_moments_local(account, before, query)?;
                for item in page.data {
                    consume(item.moment)?;
                }
                let Some(next) = page.next_cursor else { break };
                before = Some(next);
            }
        }
        drop(consume);
        lease.check()?;
        html.push_str("</html>");
        for (name, bytes) in [
            ("index.html", html.as_bytes()),
            ("moments.jsonl", jsonl.as_slice()),
        ] {
            zip.start_file(name, options).map_err(|_| failure())?;
            zip.write_all(bytes).map_err(|_| failure())?;
        }
        zip.start_file("manifest.json", options)
            .map_err(|_| failure())?;
        serde_json::to_writer(
            &mut zip,
            &serde_json::json!({"summary":summary,"media":outcomes}),
        )
        .map_err(|_| failure())?;
        lease.check()?;
        zip.finish().map_err(|_| failure())?;
        Ok(summary)
    }
}
