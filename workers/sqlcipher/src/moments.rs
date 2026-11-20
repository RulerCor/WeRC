//! Local cached timeline, ordered by unsigned ID with rowid as a stable tie-breaker.
use crate::{live::ReadView, moments_protocol::*};
use rusqlite::{Connection, params};
use std::path::Path;
use zeroize::Zeroizing;

#[cfg(test)]
fn query(
    c: &Connection,
    after: Option<&LiveMomentAnchor>,
) -> Result<LiveMomentWindow, &'static str> {
    query_with_limit(c, after, LIVE_MOMENT_MAX_ROWS)
}
fn query_with_limit(
    c: &Connection,
    after: Option<&LiveMomentAnchor>,
    limit: usize,
) -> Result<LiveMomentWindow, &'static str> {
    if !matches!(limit, LIVE_MOMENT_MAX_ROWS | LIVE_MOMENT_SCAN_ROWS) {
        return Err("LIVE_MOMENT_LIMIT");
    }
    let boundary = after
        .map(|a| a.key().ok_or("LIVE_MOMENT_ANCHOR_INVALID"))
        .transpose()?;
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='SnsTimeLine')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "LIVE_MOMENT_SCHEMA_UNSUPPORTED")?;
    if !exists {
        return Err("LIVE_MOMENT_SCHEMA_UNSUPPORTED");
    }
    if let Some(a) = after {
        let (id, row) = boundary.ok_or("LIVE_MOMENT_ANCHOR_INVALID")?;
        let matches: bool=c.query_row("SELECT EXISTS(SELECT 1 FROM SnsTimeLine WHERE rowid=?1 AND tid=?2 AND user_name=?3)",params![row,id as i64,a.username],|r|r.get(0)).map_err(|_|"LIVE_MOMENT_SCHEMA_UNSUPPORTED")?;
        if !matches {
            return Err("LIVE_MOMENT_ANCHOR_CHANGED");
        }
    }
    let mut statement=c.prepare("SELECT rowid,tid,
        CASE WHEN typeof(user_name)='text' AND length(CAST(user_name AS BLOB)) BETWEEN 1 AND 256 THEN user_name END,
        CASE WHEN typeof(content)='text' AND length(CAST(content AS BLOB))<=65536 THEN content END,
        CASE WHEN content IS NULL THEN 'missing' WHEN typeof(content)!='text' THEN 'unsupported' WHEN length(CAST(content AS BLOB))>65536 THEN 'oversized' ELSE 'parsed' END
        FROM SnsTimeLine
        WHERE (?1 IS NULL OR (tid<0)<?1 OR ((tid<0)=?1 AND (tid<?2 OR (tid=?2 AND rowid<?3))))
        ORDER BY (tid<0) DESC,tid DESC,rowid DESC LIMIT ?4").map_err(|_|"LIVE_MOMENT_SCHEMA_UNSUPPORTED")?;
    let signed = boundary.map(|b| b.0 as i64);
    let mut rows = statement
        .query(params![
            signed.map(|i| i < 0),
            signed,
            boundary.map(|b| b.1),
            limit as i64 + 1
        ])
        .map_err(|_| "LIVE_MOMENT_READ_FAILED")?;
    let mut result = LiveMomentWindow {
        schema_version: 1,
        rows: Vec::new(),
        has_more: false,
        read_only: true,
        readonly_shm: true,
    };
    while let Some(row) = rows.next().map_err(|_| "LIVE_MOMENT_READ_FAILED")? {
        if result.rows.len() == limit {
            result.has_more = true;
            break;
        }
        let mut item = LiveMoment {
            anchor: LiveMomentAnchor {
                row_id: row
                    .get::<_, i64>(0)
                    .map_err(|_| "LIVE_MOMENT_INVALID")?
                    .to_string(),
                id: (row.get::<_, i64>(1).map_err(|_| "LIVE_MOMENT_INVALID")? as u64).to_string(),
                username: row.get(2).map_err(|_| "LIVE_MOMENT_INVALID")?,
            },
            content: None,
            content_status: row.get(4).map_err(|_| "LIVE_MOMENT_INVALID")?,
        };
        if let Some(xml) = row
            .get::<_, Option<String>>(3)
            .map_err(|_| "LIVE_MOMENT_INVALID")?
        {
            let xml = Zeroizing::new(xml);
            match parse_content(&xml, &item.anchor) {
                Ok(content) => item.content = Some(content),
                Err(_) => item.content_status = "invalid".into(),
            }
        }
        if !item.valid() {
            return Err("LIVE_MOMENT_INVALID");
        }
        result.rows.push(item);
    }
    if !result.valid_with_limit(after, limit) {
        return Err("LIVE_MOMENT_INVALID");
    }
    let encoded = Zeroizing::new(serde_json::to_vec(&result).map_err(|_| "LIVE_MOMENT_INVALID")?);
    if encoded.len()
        > if limit == LIVE_MOMENT_MAX_ROWS {
            LIVE_MOMENT_MAX_BYTES
        } else {
            LIVE_MOMENT_SCAN_BYTES
        }
    {
        return Err("LIVE_MOMENT_WINDOW_LIMIT");
    }
    Ok(result)
}
pub(crate) fn read_batch(
    path: &Path,
    key: &str,
    after: Option<&LiveMomentAnchor>,
    limit: usize,
) -> Result<LiveMomentWindow, &'static str> {
    let view = ReadView::open(path, key)?;
    let result = query_with_limit(&view.connection, after, limit)?;
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
    fn scan_batches_keep_exact_continuation_and_ui_limit() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE SnsTimeLine(tid INTEGER,user_name TEXT,content TEXT)")
            .unwrap();
        for id in 1..=601i64 {
            c.execute("INSERT INTO SnsTimeLine VALUES(?1,'owned',NULL)", [id])
                .unwrap();
        }
        let first = query_with_limit(&c, None, LIVE_MOMENT_SCAN_ROWS).unwrap();
        assert_eq!(first.rows.len(), 300);
        assert!(first.has_more);
        assert!(!first.valid(None));
        let second = query_with_limit(
            &c,
            Some(&first.rows.last().unwrap().anchor),
            LIVE_MOMENT_SCAN_ROWS,
        )
        .unwrap();
        assert_eq!(second.rows.len(), 300);
        assert_eq!(second.rows[0].anchor.id, "301");
        let last = query_with_limit(
            &c,
            Some(&second.rows.last().unwrap().anchor),
            LIVE_MOMENT_SCAN_ROWS,
        )
        .unwrap();
        assert_eq!(last.rows.len(), 1);
        assert_eq!(last.rows[0].anchor.id, "1");
        assert!(!last.has_more);
        assert_eq!(query(&c, None).unwrap().rows.len(), 30);
        assert!(query_with_limit(&c, None, 301).is_err());
    }
    #[test]
    fn unsigned_pages_rowid_ties_changed_anchor_and_unknown_content() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE SnsTimeLine(tid INTEGER,user_name TEXT,content TEXT,pack_info_buf TEXT)",
        )
        .unwrap();
        for id in 1..=35u64 {
            let tid = if id > 30 { u64::MAX - id } else { id };
            c.execute(
                "INSERT INTO SnsTimeLine VALUES(?1,'owned',NULL,'secret')",
                [tid as i64],
            )
            .unwrap();
        }
        let first = query(&c, None).unwrap();
        assert_eq!(first.rows.len(), 30);
        assert!(first.has_more);
        assert_eq!(first.rows[0].anchor.id, (u64::MAX - 31).to_string());
        let a = &first.rows.last().unwrap().anchor;
        let second = query(&c, Some(a)).unwrap();
        assert_eq!(second.rows.len(), 5);
        assert!(!second.has_more);
        assert_eq!(second.rows.last().unwrap().anchor.id, "1");
        c.execute(
            "UPDATE SnsTimeLine SET user_name='changed' WHERE rowid=?1",
            [a.key().unwrap().1],
        )
        .unwrap();
        assert!(matches!(
            query(&c, Some(a)),
            Err("LIVE_MOMENT_ANCHOR_CHANGED")
        ));
        c.execute("UPDATE SnsTimeLine SET content=?1", ["x".repeat(65537)])
            .unwrap();
        assert!(
            query(&c, None)
                .unwrap()
                .rows
                .iter()
                .all(|r| r.content_status == "oversized")
        );
        c.execute("UPDATE SnsTimeLine SET content=x'ff'", [])
            .unwrap();
        assert!(
            query(&c, None)
                .unwrap()
                .rows
                .iter()
                .all(|r| r.content_status == "unsupported")
        );
        c.execute("UPDATE SnsTimeLine SET content='<broken>'", [])
            .unwrap();
        assert!(
            query(&c, None)
                .unwrap()
                .rows
                .iter()
                .all(|r| r.content_status == "invalid")
        );
        c.execute("UPDATE SnsTimeLine SET tid=-1", []).unwrap();
        let first = query(&c, None).unwrap();
        let second = query(&c, Some(&first.rows.last().unwrap().anchor)).unwrap();
        assert_eq!(second.rows.len(), 5);
        c.execute("UPDATE SnsTimeLine SET tid=0", []).unwrap();
        assert!(query(&c, None).is_err());
    }
}

/// Exact row and author identity are checked again before exposing a transport descriptor.
pub(crate) fn media(
    path: &Path,
    key: &str,
    selection: &MomentMediaSelection,
) -> Result<MomentMediaDescriptor, &'static str> {
    if !selection.valid() {
        return Err("MOMENT_MEDIA_INVALID");
    }
    let view = ReadView::open(path, key)?;
    let (id, row) = selection.anchor.key().ok_or("MOMENT_MEDIA_INVALID")?;
    let xml:String=view.connection.query_row("SELECT content FROM SnsTimeLine WHERE rowid=?1 AND tid=?2 AND user_name=?3 AND typeof(content)='text' AND length(CAST(content AS BLOB))<=65536",params![row,id as i64,selection.anchor.username],|r|r.get(0)).map_err(|_|"MOMENT_MEDIA_MISSING")?;
    let xml = Zeroizing::new(xml);
    let descriptor = media_descriptor(&xml, selection)?;
    view.check_deadline()?;
    view.connection
        .execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    Ok(descriptor)
}
