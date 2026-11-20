use super::*;
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
pub(super) fn render(
    selection: &LiveExportSelection,
    rows: &[Row],
    format: LiveExportFormat,
    sources: usize,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let mut out = crate::directory_export::Output(Zeroizing::new(Vec::new()));
    (||->std::io::Result<()> {
        if matches!(format,LiveExportFormat::Html){
            out.write_all(r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"><title>WeRC 会话导出</title><style>body{font:16px/1.6 system-ui,sans-serif;max-width:860px;margin:32px auto;padding:0 18px;color:#202632}article{border-top:1px solid #ddd;padding:16px 0}pre{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit}small{color:#626c78}h1{overflow-wrap:anywhere}details{font-size:13px;overflow-wrap:anywhere}</style></head><body>"#.as_bytes())?;
            write!(out,"<h1>{}</h1><p>{}条消息 · {}个所选来源 · 时间为UTC</p><p>本地来源的独立续页读取，不代表服务器完整历史。未包含媒体附件或原始XML。</p>",escape(&selection.username),rows.len(),sources)?;
            for row in rows {
                let m=&row.message;
                let time=chrono::DateTime::from_timestamp(m.timestamp,0).map(|d|d.to_rfc3339()).unwrap_or_default();
                write!(out,"<article><small>{} · {}</small><pre>{}</pre><details><summary>消息信息</summary>来源 {} · 本地ID {} · 服务器ID {} · 类型 {} · 内容状态 {}</details></article>",escape(&time),escape(m.sender_username.as_deref().unwrap_or("未知发送者")),escape(m.text.as_deref().unwrap_or("[无可导出的文字，请在软件内查看对应媒体或卡片]")),row.source_index,escape(&m.local_id),escape(m.server_id.as_deref().unwrap_or("未知")),escape(&m.message_type),escape(&m.content_status))?;
            }
            out.write_all(b"</body></html>")?;
        }else{
            writeln!(out,"# WeRC 会话导出\n\n{}条消息 · {}个所选来源 · 时间为UTC\n\n本地来源的独立续页读取，不代表服务器完整历史。未包含媒体附件或原始XML。\n",rows.len(),sources)?;
            for row in rows {
                let m=&row.message;
                let time=chrono::DateTime::from_timestamp(m.timestamp,0).map(|d|d.to_rfc3339()).unwrap_or_default();
                let text=Zeroizing::new(format!("会话：{}\n时间：{}\n发送者：{}\n来源：{} / 本地ID：{} / 服务器ID：{} / 类型：{} / 内容状态：{}\n{}",selection.username,time,m.sender_username.as_deref().unwrap_or("未知发送者"),row.source_index,m.local_id,m.server_id.as_deref().unwrap_or("未知"),m.message_type,m.content_status,m.text.as_deref().unwrap_or("[无可导出的文字，请在软件内查看对应媒体或卡片]")));
                let normalized=Zeroizing::new(text.replace("\r\n","\n").replace('\r',"\n"));
                for line in normalized.split('\n'){writeln!(out,"    {line}")?;}
                out.write_all(b"\n---\n\n")?;
            }
        }
        Ok(())
    })().map_err(|_|limit())?;
    Ok(out.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readable_exports_escape_source_content_and_preserve_exact_ids() {
        let selection = LiveExportSelection {
            username: "owned<&>".into(),
            query: LiveMessageQuery::default(),
        };
        let rows = [Row {
            source_index: 2,
            message: LiveMessage {
                at_user_list: None,
                local_id: "9007199254740993".into(),
                server_id: Some("9223372036854775806".into()),
                timestamp: 1700000000,
                message_type: "1".into(),
                text: Some("<script>alert(1)</script>\r[link](https://invalid.example)".into()),
                metadata: None,
                sender_username: Some("<sender>".into()),
                content_status: "text".into(),
            },
        }];
        let bytes = render(&selection, &rows, LiveExportFormat::Html, 2).unwrap();
        let html = std::str::from_utf8(&bytes).unwrap();
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<sender>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("9223372036854775806"));
        assert!(html.contains("default-src 'none'"));
        let bytes = render(&selection, &rows, LiveExportFormat::Md, 2).unwrap();
        let md = std::str::from_utf8(&bytes).unwrap();
        assert!(md.contains("\n    <script>"));
        assert!(md.contains("\n    [link]"));
        assert!(!md.contains("\n[link]"));
        assert!(!md.contains('\r'));
        assert!(md.contains("9007199254740993"));
    }
}
