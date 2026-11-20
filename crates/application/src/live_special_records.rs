use crate::{Application, history::QueryLease, live_search::SearchTicket};
use serde::Serialize;
use werc_domain::{AppError, AppResult, special_records::*};
#[derive(Serialize)]
pub struct SpecialRecordPage {
    pub rows: Vec<SpecialRecord>,
    pub next: Option<String>,
    pub query: RecordQuery,
    pub scanned: usize,
    pub expires_in_seconds: u64,
    pub cross_page_snapshot: bool,
    pub mode: &'static str,
}
fn stale() -> AppError {
    AppError::new("CURSOR_STALE", "记录来源或登录状态已变化，请重新读取")
}
impl Application {
    pub fn live_special_records_local(
        &self,
        account: &str,
        view: &str,
        query: Option<RecordQuery>,
        cursor: Option<&str>,
        request: &str,
    ) -> AppResult<SpecialRecordPage> {
        let lease = self.source_lease(request)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        self.special_records_page(account, view, query, cursor, &lease)
    }
    fn special_records_page(
        &self,
        account: &str,
        view: &str,
        query: Option<RecordQuery>,
        cursor: Option<&str>,
        lease: &QueryLease<'_>,
    ) -> AppResult<SpecialRecordPage> {
        if query.is_some() == cursor.is_some() || query.as_ref().is_some_and(|q| !q.valid()) {
            return Err(AppError::new(
                "INVALID_ARGUMENT",
                "首次读取需要筛选条件，继续读取只需游标",
            ));
        }
        lease.check()?;
        let (binding, source, expected, selection) =
            self.with_search_parent(account, view, |binding, parent, sources, _| {
                let (expected, selection) = if let Some(c) = cursor {
                    match parent.open_search(binding, view, c)? {
                        SearchTicket::SpecialRecords {
                            fingerprint,
                            selection,
                        } => (Some(fingerprint), selection),
                        _ => return Err(stale()),
                    }
                } else {
                    (
                        None,
                        RecordSelection {
                            query: query.ok_or_else(stale)?,
                            after: None,
                        },
                    )
                };
                if !selection.valid() {
                    return Err(stale());
                }
                Ok((
                    binding.to_owned(),
                    sources.first().ok_or_else(stale)?.clone(),
                    expected,
                    selection,
                ))
            })?;
        let (fingerprint, window) = self.provider.read_live_special_records(
            account,
            &source.database,
            &source.fingerprint,
            &selection,
            expected.as_deref(),
            &lease.cancel,
        )?;
        lease.check()?;
        if fingerprint.len() != 64
            || !fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            || expected.as_ref().is_some_and(|v| v != &fingerprint)
            || !window.valid(&selection)
            || zeroize::Zeroizing::new(
                serde_json::to_vec(&window).map_err(|_| AppError::internal())?,
            )
            .len()
                > MAX_BYTES
        {
            return Err(stale());
        }
        self.with_search_parent(account, view, |current, parent, sources, _| {
            if current != binding
                || sources
                    .first()
                    .is_none_or(|s| s.fingerprint != source.fingerprint)
            {
                return Err(stale());
            }
            let expires = parent.check(current, view)?;
            let next = window
                .next
                .map(|after| {
                    parent.seal_search(
                        current,
                        view,
                        &SearchTicket::SpecialRecords {
                            fingerprint,
                            selection: RecordSelection {
                                query: selection.query.clone(),
                                after: Some(after),
                            },
                        },
                    )
                })
                .transpose()?;
            lease.check()?;
            Ok(SpecialRecordPage {
                rows: window.rows,
                next,
                query: selection.query,
                scanned: window.scanned,
                expires_in_seconds: expires,
                cross_page_snapshot: false,
                mode: "local_general_records",
            })
        })
    }
    pub fn export_special_records_local<T>(
        &self,
        account: &str,
        view: &str,
        query: RecordQuery,
        request: &str,
        format: &str,
        publish: impl FnOnce(&[u8], usize) -> AppResult<T>,
    ) -> AppResult<T> {
        if !matches!(format, "json" | "csv" | "txt" | "xlsx") {
            return Err(AppError::new("INVALID_ARGUMENT", "记录导出格式无效"));
        }
        let lease = self.source_lease(request)?;
        let _guard = self.history_guard.read(|| lease.check())?;
        let mut cursor = None;
        let mut first = Some(query);
        let mut rows = Vec::new();
        let mut retained = 0usize;
        loop {
            let page =
                self.special_records_page(account, view, first.take(), cursor.as_deref(), &lease)?;
            retained += zeroize::Zeroizing::new(
                serde_json::to_vec(&page.rows).map_err(|_| AppError::internal())?,
            )
            .len();
            if retained > 32 * 1024 * 1024 || rows.len() + page.rows.len() > 50000 {
                return Err(AppError::new("EXPORT_LIMIT", "记录过多，请缩小日期范围"));
            }
            rows.extend(page.rows);
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        lease.check()?;
        let data = encode(&rows, format)?;
        lease.check()?;
        publish(&data, rows.len())
    }
}
fn encode(rows: &[SpecialRecord], format: &str) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
    let fail = || AppError::new("EXPORT_FAILED", "记录导出失败");
    if format == "json" {
        return serde_json::to_vec_pretty(&serde_json::json!({"mode":"local_general_records","cross_page_snapshot":false,"rows":rows})).map(zeroize::Zeroizing::new).map_err(|_|fail());
    }
    let headers = [
        "来源类型",
        "本地行编号",
        "时间戳",
        "会话或账号",
        "方向",
        "正文",
        "原始状态与详情",
    ];
    let cells = |r: &SpecialRecord| -> [String; 7] {
        [
            serde_json::to_string(&r.source)
                .unwrap_or_default()
                .trim_matches('"')
                .into(),
            r.anchor.row_id.clone(),
            r.timestamp.map(|v| v.to_string()).unwrap_or_default(),
            r.username.clone().unwrap_or_default(),
            r.direction.clone(),
            r.text.clone(),
            r.fields
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ]
    };
    if format == "xlsx" {
        let mut book = rust_xlsxwriter::Workbook::new();
        let sheet = book.add_worksheet();
        for (i, h) in headers.iter().enumerate() {
            sheet.write_string(0, i as u16, *h).map_err(|_| fail())?;
        }
        for (n, r) in rows.iter().enumerate() {
            for (i, c) in cells(r).iter().enumerate() {
                sheet
                    .write_string(n as u32 + 1, i as u16, c)
                    .map_err(|_| fail())?;
            }
        }
        return book
            .save_to_buffer()
            .map(zeroize::Zeroizing::new)
            .map_err(|_| fail());
    }
    let mut output = String::new();
    let mut write = |fields: Vec<String>| {
        if format == "csv" {
            output.push_str(
                &fields
                    .iter()
                    .map(|s| {
                        format!(
                            "\"{}{}\"",
                            if s.trim_start().starts_with(['=', '+', '-', '@'])
                                || s.starts_with(['\t', '\r'])
                            {
                                "'"
                            } else {
                                ""
                            },
                            s.replace('"', "\"\"")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
            output.push_str("\r\n");
        } else {
            for (h, v) in headers.iter().zip(fields) {
                output.push_str(&format!("{h}: {v}\n"));
            }
            output.push('\n');
        }
    };
    if format == "csv" {
        write(headers.iter().map(|s| s.to_string()).collect());
    }
    for row in rows {
        write(cells(row).into());
    }
    Ok(zeroize::Zeroizing::new(output.into_bytes()))
}
