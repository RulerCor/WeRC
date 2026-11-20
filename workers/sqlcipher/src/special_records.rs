use crate::{live::ReadView, special_records_protocol::*};
use rusqlite::{Connection, params};
use std::path::Path;
fn layout(
    source: RecordSource,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    match source {
        RecordSource::FriendRequest => (
            "FMessageTable",
            "timestamp_",
            "user_name_",
            "content_",
            "is_sender_",
            "type_,scene_",
        ),
        RecordSource::SearchChat => (
            "SearchRecent",
            "last_click_time",
            "username",
            "query",
            "NULL",
            "score,NULL",
        ),
        RecordSource::SearchWeb => (
            "websearch_record",
            "create_time",
            "NULL",
            "keyword",
            "NULL",
            "NULL,NULL",
        ),
        RecordSource::SearchBrand => (
            "brand_search_record",
            "create_time",
            "NULL",
            "keyword",
            "NULL",
            "NULL,NULL",
        ),
        RecordSource::Revoke => (
            "revokemessage",
            "revoke_time",
            "to_user_name",
            "content",
            "NULL",
            "message_type,svr_id",
        ),
        RecordSource::Transfer => (
            "transferTable",
            "last_update_time",
            "session_name",
            "NULL",
            "NULL",
            "pay_sub_type,message_server_id",
        ),
        RecordSource::RedPacket => (
            "redEnvelopeTable",
            "0",
            "session_name",
            "NULL",
            "NULL",
            "hb_status,receive_status",
        ),
    }
}
fn bounded(column: &str, limit: usize) -> String {
    format!(
        "CASE WHEN typeof({column})='text' AND length(CAST({column} AS BLOB))<={limit} THEN {column} END"
    )
}
fn query(c: &Connection, s: &RecordSelection) -> Result<RecordWindow, &'static str> {
    if !s.valid() {
        return Err("RECORD_SELECTION_INVALID");
    }
    let (table, time, user, body, direction, fields) = layout(s.query.source);
    let mut out = RecordWindow {
        rows: vec![],
        next: None,
        scanned: 0,
        read_only: true,
        readonly_shm: true,
    };
    let after = s.after.as_ref().and_then(RecordAnchor::key);
    if let Some((time_value, row)) = after {
        let exists: bool = c
            .query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM {table} WHERE rowid=?1 AND coalesce({time},0)=?2)"
                ),
                params![row, time_value],
                |r| r.get(0),
            )
            .map_err(|_| "RECORD_SCHEMA_UNSUPPORTED")?;
        if !exists {
            return Err("RECORD_ANCHOR_CHANGED");
        }
    }
    let sql = format!(
        "SELECT rowid,coalesce({time},0),{},{},{direction},{fields} FROM {table} WHERE (?1 IS NULL OR coalesce({time},0)<?1 OR (coalesce({time},0)=?1 AND rowid<?2)) ORDER BY coalesce({time},0) DESC,rowid DESC LIMIT 301",
        bounded(user, 256),
        bounded(body, 65536)
    );
    let mut stmt = c.prepare(&sql).map_err(|_| "RECORD_SCHEMA_UNSUPPORTED")?;
    let mut rows = stmt
        .query(params![after.map(|a| a.0), after.map(|a| a.1)])
        .map_err(|_| "RECORD_READ_FAILED")?;
    let mut last = None;
    let needle = s.query.text.to_lowercase();
    while let Some(r) = rows.next().map_err(|_| "RECORD_READ_FAILED")? {
        if out.scanned == 300 || out.rows.len() == MAX_ROWS {
            out.next = last;
            break;
        }
        let raw_time: i64 = r.get(1).map_err(|_| "RECORD_ROW_INVALID")?;
        let anchor = RecordAnchor {
            row_id: r
                .get::<_, i64>(0)
                .map_err(|_| "RECORD_ROW_INVALID")?
                .to_string(),
            raw_time,
        };
        last = Some(anchor.clone());
        out.scanned += 1;
        let seconds = if raw_time > 100_000_000_000 {
            raw_time / 1000
        } else {
            raw_time
        };
        let timestamp = (seconds > 0 && seconds <= 253402300799).then_some(seconds);
        if s.query
            .from
            .is_some_and(|v| timestamp.is_none_or(|t| t < v))
            || s.query
                .until
                .is_some_and(|v| timestamp.is_none_or(|t| t >= v))
        {
            continue;
        }
        let username = r
            .get::<_, Option<String>>(2)
            .map_err(|_| "RECORD_ROW_INVALID")?
            .and_then(|s| text(&s, 256));
        let body = zeroize::Zeroizing::new(
            r.get::<_, Option<String>>(3)
                .map_err(|_| "RECORD_ROW_INVALID")?
                .unwrap_or_default(),
        );
        let display = if matches!(s.query.source, RecordSource::FriendRequest) {
            request_text(&body)
        } else if body.trim_start().starts_with('<') {
            None
        } else {
            text(&body, 8192)
        };
        let direction = match r
            .get::<_, Option<i64>>(4)
            .map_err(|_| "RECORD_ROW_INVALID")?
        {
            Some(0) => "incoming",
            Some(1) => "outgoing",
            _ => "unknown",
        }
        .into();
        let mut fields = std::collections::BTreeMap::new();
        for (index, label) in match s.query.source {
            RecordSource::FriendRequest => [(5, "申请类型原码"), (6, "来源场景原码")],
            RecordSource::SearchChat => [(5, "排序分值"), (6, "")],
            RecordSource::Revoke => [(5, "消息类型"), (6, "服务端消息编号")],
            RecordSource::Transfer => [(5, "支付子类型原码"), (6, "服务端消息编号")],
            RecordSource::RedPacket => [(5, "红包状态原码"), (6, "接收状态原码")],
            _ => [(5, ""), (6, "")],
        } {
            if !label.is_empty() {
                if let Some(value) = r
                    .get::<_, Option<i64>>(index)
                    .map_err(|_| "RECORD_ROW_INVALID")?
                {
                    fields.insert(
                        label.into(),
                        if label == "服务端消息编号" {
                            (value as u64).to_string()
                        } else {
                            value.to_string()
                        },
                    );
                }
            }
        }
        if !body.is_empty() && display.is_none() {
            fields.insert("正文状态".into(), "未解析".into());
        }
        let record = SpecialRecord {
            anchor,
            source: s.query.source,
            timestamp,
            username,
            text: display.unwrap_or_default(),
            direction,
            fields,
        };
        if !record.valid() {
            return Err("RECORD_ROW_INVALID");
        }
        if !needle.is_empty()
            && !format!(
                "{}\n{}",
                record.username.as_deref().unwrap_or(""),
                record.text
            )
            .to_lowercase()
            .contains(&needle)
        {
            continue;
        }
        out.rows.push(record);
    }
    if !out.valid(s)
        || serde_json::to_vec(&out)
            .map_err(|_| "RECORD_INVALID")?
            .len()
            > MAX_BYTES
    {
        return Err("RECORD_WINDOW_LIMIT");
    }
    Ok(out)
}
pub(crate) fn read(
    path: &Path,
    key: &str,
    s: &RecordSelection,
) -> Result<RecordWindow, &'static str> {
    let view = ReadView::open(path, key)?;
    let result = query(&view.connection, s)?;
    view.check_deadline()?;
    view.connection
        .execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_and_search_pagination_never_export_tickets_or_raw_payload() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE FMessageTable(user_name_ TEXT,timestamp_ INTEGER,content_ TEXT,is_sender_ INTEGER,type_ INTEGER,scene_ INTEGER,ticket_ TEXT);CREATE TABLE SearchRecent(username TEXT,query TEXT,score INTEGER,last_click_time INTEGER)").unwrap();
        c.execute(
            "INSERT INTO FMessageTable VALUES('owned',1700000000,?1,0,3,6,'secret-ticket')",
            ["<msg content='自动化测试申请' ticket='secret-ticket'/>"],
        )
        .unwrap();
        let selection = |source| RecordSelection {
            query: RecordQuery {
                source,
                text: String::new(),
                from: None,
                until: None,
            },
            after: None,
        };
        let p = query(&c, &selection(RecordSource::FriendRequest)).unwrap();
        assert_eq!(p.rows[0].text, "自动化测试申请");
        assert!(!serde_json::to_string(&p).unwrap().contains("secret-ticket"));
        for n in 1..=101 {
            c.execute(
                "INSERT INTO SearchRecent VALUES('owned','自动化测试',1,?1)",
                [1700000000000i64 + n],
            )
            .unwrap();
        }
        let mut s = selection(RecordSource::SearchChat);
        let p = query(&c, &s).unwrap();
        assert_eq!(p.rows.len(), 100);
        assert_eq!(p.rows[0].timestamp, Some(1700000000));
        s.after = p.next;
        let last = query(&c, &s).unwrap();
        assert_eq!(last.rows.len(), 1);
        assert!(last.next.is_none());
    }
}
