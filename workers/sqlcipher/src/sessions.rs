//! Recent local sessions, without reading drafts or mutating unread state.
use crate::{live::ReadView, live_protocol::*};
use rusqlite::{Connection, params};
use std::path::Path;
use zeroize::Zeroizing;
fn query_filtered(
    c: &Connection,
    after: Option<&LiveSessionAnchor>,
    search_query: Option<&str>,
) -> Result<LiveSessionWindow, &'static str> {
    if search_query.is_some_and(|q| !valid_session_query(q)) {
        return Err("LIVE_QUERY_INVALID");
    }
    let needle = search_query.map(|q| Zeroizing::new(q.to_lowercase()));
    c.create_scalar_function(
        "werc_session_matches",
        2,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        move |context| {
            let Some(needle) = needle.as_ref() else {
                return Ok(true);
            };
            let username = match context.get_raw(0) {
                rusqlite::types::ValueRef::Text(bytes)
                    if !bytes.is_empty() && bytes.len() <= 256 =>
                {
                    std::str::from_utf8(bytes).map_err(|_| rusqlite::Error::InvalidQuery)?
                }
                _ => return Err(rusqlite::Error::InvalidQuery),
            };
            if username.chars().any(char::is_control) {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let summary = match context.get_raw(1) {
                rusqlite::types::ValueRef::Text(bytes) if bytes.len() <= 4096 => {
                    std::str::from_utf8(bytes)
                        .ok()
                        .filter(|s| !s.contains('\0'))
                }
                _ => None,
            };
            Ok(username.to_lowercase().contains(needle.as_str())
                || summary.is_some_and(|s| s.to_lowercase().contains(needle.as_str())))
        },
    )
    .map_err(|_| "LIVE_QUERY_UNAVAILABLE")?;
    let boundary = after
        .map(|a| a.key().ok_or("LIVE_SESSION_ANCHOR_INVALID"))
        .transpose()?;
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='SessionTable')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "LIVE_SESSION_SCHEMA_UNSUPPORTED")?;
    if !exists {
        return Err("LIVE_SESSION_SCHEMA_UNSUPPORTED");
    }
    if let Some(anchor) = after {
        let matches:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM SessionTable WHERE rowid=?1 AND username=?2 AND coalesce(sort_timestamp,last_timestamp,0)=?3)",params![boundary.unwrap().1,anchor.username,anchor.sort_timestamp],|r|r.get(0)).map_err(|_|"LIVE_SESSION_SCHEMA_UNSUPPORTED")?;
        if !matches {
            return Err("LIVE_SESSION_ANCHOR_CHANGED");
        }
    }
    let mut statement=c.prepare("SELECT rowid,
      CASE WHEN typeof(username)='text' AND length(CAST(username AS BLOB)) BETWEEN 1 AND 256 THEN username END,
      coalesce(sort_timestamp,last_timestamp,0),
      CASE WHEN typeof(summary)='text' AND length(CAST(summary AS BLOB))<=4096 THEN summary END,
      CASE WHEN summary IS NULL THEN 'missing' WHEN typeof(summary)!='text' THEN 'invalid' WHEN length(CAST(summary AS BLOB))>4096 THEN 'oversized' ELSE 'text' END,
      last_timestamp,unread_count,is_hidden FROM SessionTable
      WHERE (?1 IS NULL OR coalesce(sort_timestamp,last_timestamp,0)<?1 OR (coalesce(sort_timestamp,last_timestamp,0)=?1 AND rowid<?2))
      AND werc_session_matches(username,summary)
      ORDER BY coalesce(sort_timestamp,last_timestamp,0) DESC,rowid DESC LIMIT ?3").map_err(|_|"LIVE_SESSION_SCHEMA_UNSUPPORTED")?;
    let mut rows = statement
        .query(params![
            boundary.map(|b| b.0),
            boundary.map(|b| b.1),
            LIVE_SESSION_MAX_ROWS as i64 + 1
        ])
        .map_err(|_| "LIVE_SESSION_READ_FAILED")?;
    let mut result = LiveSessionWindow {
        schema_version: 1,
        rows: Vec::new(),
        has_more: false,
        read_only: true,
        readonly_shm: true,
    };
    while let Some(row) = rows.next().map_err(|_| "LIVE_SESSION_READ_FAILED")? {
        if result.rows.len() == LIVE_SESSION_MAX_ROWS {
            result.has_more = true;
            break;
        }
        let mut item = LiveSession {
            anchor: LiveSessionAnchor {
                row_id: row
                    .get::<_, i64>(0)
                    .map_err(|_| "LIVE_SESSION_INVALID")?
                    .to_string(),
                username: row.get(1).map_err(|_| "LIVE_SESSION_INVALID")?,
                sort_timestamp: row.get(2).map_err(|_| "LIVE_SESSION_INVALID")?,
            },
            summary: row.get(3).map_err(|_| "LIVE_SESSION_INVALID")?,
            summary_status: row.get(4).map_err(|_| "LIVE_SESSION_INVALID")?,
            last_timestamp: row.get(5).map_err(|_| "LIVE_SESSION_INVALID")?,
            unread_count: row.get(6).map_err(|_| "LIVE_SESSION_INVALID")?,
            is_hidden: row.get(7).map_err(|_| "LIVE_SESSION_INVALID")?,
        };
        if item.summary.as_ref().is_some_and(|s| s.contains('\0')) {
            item.summary = None;
            item.summary_status = "invalid".into();
        }
        if !item.valid() {
            return Err("LIVE_SESSION_INVALID");
        }
        result.rows.push(item);
    }
    if !result.valid(after) {
        return Err("LIVE_SESSION_INVALID");
    }
    let bytes = Zeroizing::new(serde_json::to_vec(&result).map_err(|_| "LIVE_SESSION_INVALID")?);
    if bytes.len() > LIVE_SESSION_MAX_BYTES {
        return Err("LIVE_SESSION_WINDOW_LIMIT");
    }
    Ok(result)
}
pub(crate) fn read_filtered(
    path: &Path,
    key: &str,
    after: Option<&LiveSessionAnchor>,
    search_query: Option<&str>,
) -> Result<LiveSessionWindow, &'static str> {
    let view = ReadView::open(path, key)?;
    let result = query_filtered(&view.connection, after, search_query);
    view.check_deadline()?;
    let result = result?;
    view.connection
        .execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    Ok(result)
}
#[cfg(test)]
fn query(
    c: &Connection,
    after: Option<&LiveSessionAnchor>,
) -> Result<LiveSessionWindow, &'static str> {
    query_filtered(c, after, None)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_pages_ties_large_ids_and_changed_boundary() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE SessionTable(username TEXT,sort_timestamp INTEGER,last_timestamp INTEGER,summary TEXT,unread_count INTEGER,is_hidden INTEGER,draft TEXT)").unwrap();
        for n in 1..=201 {
            c.execute("INSERT INTO SessionTable(rowid,username,sort_timestamp,last_timestamp,summary,unread_count,is_hidden,draft) VALUES(?1,?2,20,19,'自有摘要 <>',0,0,'not exposed')",params![9007199254740992i64+n,format!("owned-session-{n}")]).unwrap();
        }
        let first = query(&c, None).unwrap();
        assert_eq!(first.rows.len(), 200);
        assert!(first.has_more);
        assert_eq!(first.rows[0].anchor.row_id, "9007199254741193");
        let anchor = &first.rows.last().unwrap().anchor;
        let last = query(&c, Some(anchor)).unwrap();
        assert_eq!(last.rows.len(), 1);
        assert!(!last.has_more);
        c.execute(
            "UPDATE SessionTable SET sort_timestamp=21 WHERE rowid=?1",
            [anchor.key().unwrap().1],
        )
        .unwrap();
        assert!(matches!(
            query(&c, Some(anchor)),
            Err("LIVE_SESSION_ANCHOR_CHANGED")
        ));
        c.execute("UPDATE SessionTable SET summary=?1", ["中".repeat(1366)])
            .unwrap();
        let p = query(&c, None).unwrap();
        assert!(
            p.rows
                .iter()
                .all(|r| r.summary.is_none() && r.summary_status == "oversized")
        );
        c.execute("UPDATE SessionTable SET summary=x'fffe'", [])
            .unwrap();
        assert!(
            query(&c, None)
                .unwrap()
                .rows
                .iter()
                .all(|r| r.summary_status == "invalid")
        );
        c.execute("UPDATE SessionTable SET username='bad'||char(0)||'id'", [])
            .unwrap();
        assert!(query(&c, None).is_err());
    }
}
