use serde::Serialize;
use std::io::{self, Write};
use werc_domain::{
    AppError, AppResult,
    history::{HistoryMessage, HistorySource},
};

const LIMIT: usize = 32 * 1024 * 1024;
#[derive(Default)]
struct Output(Vec<u8>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|v| v > LIMIT)
        {
            return Err(io::Error::other("export limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn csv(out: &mut Output, value: &str) -> io::Result<()> {
    out.write_all(b"\"")?;
    if value
        .trim_start_matches([' ', '\u{feff}', '\n'])
        .starts_with(['=', '+', '-', '@', '\t', '\r'])
    {
        out.write_all(b"'")?;
    }
    for part in value.split_inclusive('"') {
        out.write_all(part.as_bytes())?;
        if part.ends_with('"') {
            out.write_all(b"\"")?;
        }
    }
    out.write_all(b"\"")
}
#[derive(Serialize)]
struct Export<'a> {
    product: &'static str,
    account_id: &'a str,
    data_mode: &'static str,
    sources: &'a [HistorySource],
    messages: &'a [HistoryMessage],
}
#[derive(Serialize)]
struct JsonLine<'a> {
    data_mode: &'static str,
    message: &'a HistoryMessage,
}
fn render(
    out: &mut Output,
    account: &str,
    sources: &[HistorySource],
    messages: &[HistoryMessage],
    format: &str,
) -> io::Result<()> {
    match format {
        "json" => serde_json::to_writer_pretty(
            out,
            &Export {
                product: "WeRC_Mon3trBot",
                account_id: account,
                data_mode: "imported_snapshot",
                sources,
                messages,
            },
        )
        .map_err(io::Error::other),
        "jsonl" => {
            for message in messages {
                serde_json::to_writer(
                    &mut *out,
                    &JsonLine {
                        data_mode: "imported_snapshot",
                        message,
                    },
                )
                .map_err(io::Error::other)?;
                out.write_all(b"\n")?;
            }
            Ok(())
        }
        "csv" => {
            out.write_all(b"conversation_id,sender_id,timestamp,server_id,message_type,text\r\n")?;
            for m in messages {
                for (index, value) in [
                    m.conversation_id.as_str(),
                    m.sender_id.as_deref().unwrap_or_default(),
                    &m.timestamp.to_string(),
                    &m.server_id,
                    &m.message_type.to_string(),
                    m.text.as_deref().unwrap_or_default(),
                ]
                .iter()
                .enumerate()
                {
                    if index != 0 {
                        out.write_all(b",")?;
                    }
                    csv(out, value)?;
                }
                out.write_all(b"\r\n")?;
            }
            Ok(())
        }
        "txt" => {
            for (i, m) in messages.iter().enumerate() {
                if i != 0 {
                    out.write_all(b"\n\n")?;
                }
                write!(
                    out,
                    "[{}] {} / {}\n{}",
                    m.timestamp,
                    m.conversation_id,
                    m.sender_id.as_deref().unwrap_or("unknown"),
                    m.text.as_deref().unwrap_or("[binary_or_missing]")
                )?;
            }
            Ok(())
        }
        "html" => {
            out.write_all(b"<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; base-uri 'none'; form-action 'none'\"><title>WeRC history export</title></head><body><h1>WeRC_Mon3trBot</h1><p>imported_snapshot / Offline messages</p>")?;
            for m in messages {
                write!(
                    out,
                    "<article><h2>{}</h2><p>{} / {} / {}</p><pre>{}</pre></article>",
                    html(&m.conversation_id),
                    m.timestamp,
                    html(m.sender_id.as_deref().unwrap_or("unknown")),
                    html(&m.server_id),
                    html(m.text.as_deref().unwrap_or("[binary_or_missing]"))
                )?;
            }
            out.write_all(b"</body></html>")
        }
        "md" => {
            out.write_all(b"# WeRC_Mon3trBot\n\nimported_snapshot / Offline messages\n\n")?;
            for m in messages {
                // 所有来源字段使用缩进代码块，不能创建HTML、链接或Markdown指令。
                let value = format!(
                    "[{}] {} / {} / {}\n{}",
                    m.timestamp,
                    m.conversation_id,
                    m.sender_id.as_deref().unwrap_or("unknown"),
                    m.server_id,
                    m.text.as_deref().unwrap_or("[binary_or_missing]")
                );
                let value = value.replace("\r\n", "\n").replace('\r', "\n");
                for line in value.lines() {
                    writeln!(out, "    {line}")?;
                }
                out.write_all(b"\n---\n\n")?;
            }
            Ok(())
        }
        "sql" => {
            out.write_all(b"-- WeRC offline export. Never apply this to a WeChat database.\nBEGIN TRANSACTION;\nCREATE TABLE werc_export_messages (id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, sender_id TEXT, timestamp INTEGER NOT NULL, server_id TEXT NOT NULL, message_type INTEGER NOT NULL, text TEXT);\n")?;
            fn value(value: Option<&str>) -> String {
                value.map_or_else(
                    || "NULL".into(),
                    |s| format!("CAST(X'{}' AS TEXT)", hex::encode(s.as_bytes())),
                )
            }
            for m in messages {
                writeln!(
                    out,
                    "INSERT INTO werc_export_messages VALUES ({},{},{},{},{},{},{});",
                    value(Some(&m.id)),
                    value(Some(&m.conversation_id)),
                    value(m.sender_id.as_deref()),
                    m.timestamp,
                    value(Some(&m.server_id)),
                    m.message_type,
                    value(m.text.as_deref())
                )?;
            }
            out.write_all(b"COMMIT;\n")
        }
        _ => unreachable!(),
    }
}
pub(crate) fn export(
    account: &str,
    sources: &[HistorySource],
    messages: &[HistoryMessage],
    format: &str,
) -> AppResult<String> {
    if !["json", "jsonl", "csv", "txt", "html", "md", "sql"].contains(&format) {
        return Err(AppError::new(
            "INVALID_ARGUMENT",
            "支持json/jsonl/csv/txt/html/md/sql导出",
        ));
    }
    let mut output = Output::default();
    render(&mut output, account, sources, messages, format).map_err(|_| {
        AppError::new(
            "QUOTA_EXCEEDED",
            "导出结果超过32MiB或不能编码，请缩小查询范围",
        )
    })?;
    String::from_utf8(output.0).map_err(|_| AppError::internal())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_writer_refuses_before_growth() {
        let mut out = Output(vec![0; LIMIT]);
        assert!(out.write_all(b"x").is_err());
        assert_eq!(out.0.len(), LIMIT);
    }
    #[test]
    fn escapes_csv_html_markdown_and_preserves_json_lines() {
        let m=HistoryMessage { id:"id".into(),conversation_id:"<script>peer</script>".into(),sender_id:None,local_id:1,server_id:"9223372036854775806".into(),timestamp:1,sequence:0,message_type:1,text:Some("\n =HYPERLINK(\"bad\")\n<script>alert(1)</script>\n[link](https://bad.invalid)\n```\r<script>bare-return</script>".into()),content_status:"text".into() };
        let list = [m];
        let csv = export("a", &[], &list, "csv").unwrap();
        assert!(csv.contains("\"'\n =HYPERLINK"));
        let html = export("a", &[], &list, "html").unwrap();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("default-src 'none'"));
        let md = export("a", &[], &list, "md").unwrap();
        assert!(md.contains("\n    <script>"));
        assert!(!md.contains("\n<script>"));
        assert!(!md.contains('\r'));
        assert!(md.contains("\n    <script>bare-return</script>"));
        let jsonl = export("a", &[], &list, "jsonl").unwrap();
        assert_eq!(jsonl.lines().count(), 1);
        let parsed: serde_json::Value = serde_json::from_str(jsonl.trim()).unwrap();
        assert_eq!(parsed["message"]["server_id"], "9223372036854775806");
        assert!(export("a", &[], &list, "unknown").is_err());
    }
}
