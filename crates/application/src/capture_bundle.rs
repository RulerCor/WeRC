//! Portable offline archive with relative media links and saved subtitles.
use super::*;
use std::io::{Seek, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};
fn failed() -> AppError {
    AppError::new("EXPORT_WRITE_FAILED", "归档ZIP写入失败")
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
#[derive(Serialize)]
pub struct BundleSummary {
    pub rows: u64,
    pub media: u64,
    pub missing_media: u64,
    pub transcripts: u64,
    pub complete_selected_sources: bool,
}
impl Application {
    pub fn export_capture_bundle_local<W: Write + Seek>(
        &self,
        account: &str,
        id: &str,
        request_id: &str,
        out: W,
    ) -> AppResult<BundleSummary> {
        let lease = self.browse_lease(request_id)?.for_local_archive();
        let status = self.source_capture_status_local(account, id)?;
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(true);
        let mut zip = ZipWriter::new(out);
        let mut revisions = Vec::new();
        let mut files = std::collections::BTreeMap::new();
        let mut summary = BundleSummary {
            rows: 0,
            media: 0,
            missing_media: 0,
            transcripts: 0,
            complete_selected_sources: status.complete_selected_sources,
        };
        for index in 0..status.pages {
            lease.check()?;
            let page = self.source_capture_page_local(account, id, index)?;
            revisions.push(page.revision);
            for r in self.capture_media_page_local(account, id, index)? {
                if page.rows[r.row].message.content_status == "deleted_local" {
                    continue;
                }
                if let Some(media) = r.saved {
                    let ext = match media.kind {
                        crate::conversation_media::ConversationMediaKind::Image => "png",
                        crate::conversation_media::ConversationMediaKind::Voice => "wav",
                        crate::conversation_media::ConversationMediaKind::Video => "mp4",
                        crate::conversation_media::ConversationMediaKind::Emoticon => {
                            if media.mime == "image/png" {
                                "png"
                            } else {
                                "gif"
                            }
                        }
                        crate::conversation_media::ConversationMediaKind::File => "bin",
                    };
                    let path = format!("media/{index}-{}.{}", r.row, ext);
                    zip.start_file(&path, options).map_err(|_| failed())?;
                    self.export_capture_media_local(
                        account,
                        id,
                        index,
                        r.row,
                        &uuid::Uuid::new_v4().to_string(),
                        &mut zip,
                    )?;
                    files.insert((index, r.row), (path, media, r.matches_message));
                    summary.media += 1;
                } else if r.kind.is_some() {
                    summary.missing_media += 1;
                }
            }
        }
        let mut subtitles = std::collections::BTreeMap::<(u64, usize), Vec<String>>::new();
        let mut before = None;
        loop {
            lease.check()?;
            let page = self.library_page_local(account, "transcript", before)?;
            for item in page.data {
                let source =
                    item.entry.source.as_deref().and_then(|s| {
                        serde_json::from_str::<werc_domain::asr::AudioSource>(s).ok()
                    });
                if let Some(werc_domain::asr::AudioSource::Archived {
                    capture_id,
                    page,
                    row,
                }) = source
                {
                    if capture_id == id && page < status.pages {
                        let source_page = self.source_capture_page_local(account, id, page)?;
                        if source_page.revision != revisions[page as usize] {
                            return Err(AppError::new(
                                "DOCUMENT_CONFLICT",
                                "导出期间消息被编辑，请重试",
                            ));
                        }
                        if source_page
                            .rows
                            .get(row)
                            .is_none_or(|r| r.message.content_status == "deleted_local")
                        {
                            continue;
                        }
                        let path = format!("transcripts/{}.txt", summary.transcripts);
                        zip.start_file(&path, options).map_err(|_| failed())?;
                        zip.write_all(item.entry.text.as_bytes())
                            .map_err(|_| failed())?;
                        if let Some(transcript) = item.entry.transcript {
                            zip.start_file(
                                format!("transcripts/{}.srt", summary.transcripts),
                                options,
                            )
                            .map_err(|_| failed())?;
                            zip.write_all(transcript.srt().as_bytes())
                                .map_err(|_| failed())?;
                            zip.start_file(
                                format!("transcripts/{}.json", summary.transcripts),
                                options,
                            )
                            .map_err(|_| failed())?;
                            serde_json::to_writer(&mut zip, &transcript).map_err(|_| failed())?;
                        }
                        subtitles.entry((page, row)).or_default().push(path);
                        summary.transcripts += 1;
                    }
                }
            }
            before = page.next_cursor;
            if before.is_none() {
                break;
            }
        }
        zip.start_file("messages.jsonl", options)
            .map_err(|_| failed())?;
        for index in 0..status.pages {
            lease.check()?;
            let page = self.source_capture_page_local(account, id, index)?;
            if page.revision != revisions[index as usize] {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "导出期间消息被编辑，请重试",
                ));
            }
            for (row, message) in page.rows.iter().enumerate() {
                if message.message.content_status == "deleted_local" {
                    continue;
                }
                serde_json::to_writer(&mut zip,&serde_json::json!({"page":index,"row":row,"message":message,"media":files.get(&(index,row)).map(|(path,media,matches)|serde_json::json!({"path":path,"name":media.name,"sha256":media.sha256,"size":media.size,"matches_message":matches})),"transcripts":subtitles.get(&(index,row))})).map_err(|_|failed())?;
                zip.write_all(b"\n").map_err(|_| failed())?;
                summary.rows += 1;
            }
        }
        zip.start_file("index.html", options)
            .map_err(|_| failed())?;
        zip.write_all(br#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta http-equiv="Content-Security-Policy" content="default-src 'none';img-src 'self';media-src 'self';style-src 'unsafe-inline';base-uri 'none';form-action 'none'"><title>WeRC</title><style>body{font:16px system-ui;max-width:850px;margin:2rem auto;padding:1rem}article{border-bottom:1px solid #ddd;padding:1rem 0}p{white-space:pre-wrap;overflow-wrap:anywhere}img,video{max-width:100%;max-height:500px}small{color:#666}audio{max-width:100%}</style></head><body><h1>WeRC</h1>"#).map_err(|_|failed())?;
        write!(
            zip,
            "<p>消息 {} 条 · 已保存媒体 {} · 缺失媒体 {} · 字幕 {}。解压全部文件后打开本页。</p>",
            summary.rows, summary.media, summary.missing_media, summary.transcripts
        )
        .map_err(|_| failed())?;
        for index in 0..status.pages {
            lease.check()?;
            let page = self.source_capture_page_local(account, id, index)?;
            if page.revision != revisions[index as usize] {
                return Err(AppError::new(
                    "DOCUMENT_CONFLICT",
                    "导出期间消息被编辑，请重试",
                ));
            }
            for (row, r) in page.rows.iter().enumerate() {
                if r.message.content_status == "deleted_local" {
                    continue;
                }
                let marker = crate::capture_tools::local_marker(&r.message.content_status);
                if !marker.is_empty() {
                    write!(zip, "<p>{marker}</p>").map_err(|_| failed())?;
                }
                write!(
                    zip,
                    "<article><small>{} · {} · {}</small><p>{}</p>",
                    escape(&r.username),
                    r.message.timestamp,
                    escape(r.message.sender_username.as_deref().unwrap_or("未知发送者")),
                    escape(r.message.text.as_deref().unwrap_or(""))
                )
                .map_err(|_| failed())?;
                if let Some((path, m, matched)) = files.get(&(index, row)) {
                    if !matched {
                        zip.write_all("<p>此媒体对应编辑前的原消息。</p>".as_bytes())
                            .map_err(|_| failed())?;
                    }
                    let path = escape(path);
                    match m.mime.as_str() {
                        "image/png" | "image/gif" => write!(
                            zip,
                            "<img loading=\"lazy\" src=\"{path}\" alt=\"归档图片\">"
                        ),
                        "audio/wav" => write!(
                            zip,
                            "<audio controls preload=\"none\" src=\"{path}\"></audio>"
                        ),
                        "video/mp4" => write!(
                            zip,
                            "<video controls preload=\"none\" src=\"{path}\"></video>"
                        ),
                        _ => Ok(()),
                    }
                    .map_err(|_| failed())?;
                    write!(
                        zip,
                        "<p><a href=\"{path}\" download=\"{}\">保存 {}</a></p>",
                        escape(&m.name),
                        escape(&m.name)
                    )
                    .map_err(|_| failed())?;
                }
                if let Some(paths) = subtitles.get(&(index, row)) {
                    for p in paths {
                        write!(zip, "<p><a href=\"{}\">查看转写稿</a></p>", escape(p))
                            .map_err(|_| failed())?;
                    }
                }
                zip.write_all(b"</article>").map_err(|_| failed())?;
            }
        }
        zip.write_all(b"</body></html>").map_err(|_| failed())?;
        zip.start_file("summary.json", options)
            .map_err(|_| failed())?;
        serde_json::to_writer_pretty(&mut zip, &summary).map_err(|_| failed())?;
        zip.finish().map_err(|_| failed())?;
        Ok(summary)
    }
}
