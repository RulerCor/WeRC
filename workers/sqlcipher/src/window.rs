//! One short transaction materializes a bounded filehelper result window.
//! GUI pagination must use a protected result view, never keep this read lock.
use crate::{live::ReadView, live_protocol::*};
use rusqlite::types::ValueRef;
use std::path::Path;

type Result<T> = std::result::Result<T, &'static str>;

pub(crate) fn column(columns: &[String], candidates: &[&str]) -> Result<String> {
    candidates
        .iter()
        .find_map(|name| columns.iter().find(|c| c.eq_ignore_ascii_case(name)))
        .filter(|c| c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .map(|c| format!("\"{c}\""))
        .ok_or("LIVE_MESSAGE_SCHEMA_UNSUPPORTED")
}

// Optional metadata: older databases can still supply messages without this map.
// Resolve in the same read transaction and only through the local integer rowid.
fn sender_expression(c: &rusqlite::Connection, columns: &[String]) -> String {
    let Ok(sender) = column(columns, &["real_sender_id"]) else {
        return "NULL".into();
    };
    let supported = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='Name2Id') AND NOT EXISTS(SELECT 1 FROM pragma_table_info('Name2Id') WHERE lower(name) IN ('rowid','_rowid_','oid'))",
        [], |r| r.get::<_, bool>(0),
    ).unwrap_or(false) && c.prepare("SELECT rowid,user_name FROM Name2Id LIMIT 0").is_ok();
    if !supported {
        return "NULL".into();
    }
    format!(
        "(SELECT CASE WHEN typeof(n.user_name)='text' AND length(CAST(n.user_name AS BLOB)) BETWEEN 1 AND 256 THEN n.user_name ELSE NULL END FROM Name2Id AS n WHERE typeof(m.{sender})='integer' AND m.{sender}>0 AND n.rowid=m.{sender})"
    )
}

pub(crate) fn conversation_table(c: &rusqlite::Connection, username: &str) -> Result<String> {
    use md5::{Digest, Md5};
    let selected = LiveConversationSelection {
        username: username.into(),
        before: None,
        include_anchor: false,
        newer_context: false,
        query: None,
    };
    if !selected.valid() {
        return Err("LIVE_CONVERSATION_INVALID");
    }
    // Require the exact local username mapping in this same short read transaction.
    let known: bool = username == "filehelper"
        || c.query_row(
            "SELECT EXISTS(SELECT 1 FROM Name2Id WHERE user_name COLLATE BINARY=?1)",
            [username],
            |row| row.get(0),
        )
        .map_err(|_| "LIVE_IDENTITY_SCHEMA_UNSUPPORTED")?;
    if !known {
        return Err("LIVE_CONVERSATION_NOT_IN_SOURCE");
    }
    Ok(format!("Msg_{:x}", Md5::digest(username.as_bytes())))
}

pub(crate) fn read(
    path: &Path,
    key: &str,
    limit: usize,
    before: Option<&LiveAnchor>,
) -> Result<LiveWindow> {
    read_selected(path, key, limit, None, before)
}

pub(crate) fn read_selected(
    path: &Path,
    key: &str,
    limit: usize,
    selection: Option<&LiveConversationSelection>,
    legacy_before: Option<&LiveAnchor>,
) -> Result<LiveWindow> {
    let view = ReadView::open(path, key)?;
    let result = read_in_view(&view, limit, selection, legacy_before)?;
    view.connection
        .execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    Ok(result)
}
pub(crate) fn read_in_view(
    view: &ReadView,
    limit: usize,
    selection: Option<&LiveConversationSelection>,
    legacy_before: Option<&LiveAnchor>,
) -> Result<LiveWindow> {
    if selection.is_some_and(|s| !s.valid()) {
        return Err("LIVE_CONVERSATION_INVALID");
    }
    let before = selection.and_then(|s| s.before.as_ref()).or(legacy_before);
    let username = selection.map(|s| s.username.as_str());
    let filter = selection.and_then(|s| s.query.as_ref());
    let include_anchor = selection.is_some_and(|s| s.include_anchor);
    let newer = selection.is_some_and(|s| s.newer_context);
    let limit = if newer { limit.min(30) } else { limit };
    if !(1..=LIVE_WINDOW_MAX_ROWS).contains(&limit) {
        return Err("LIVE_WINDOW_LIMIT");
    }
    if filter.is_some_and(|q| !q.valid()) {
        return Err("LIVE_QUERY_INVALID");
    }
    if include_anchor && before.is_none() {
        return Err("LIVE_ANCHOR_INVALID");
    }
    let boundary = before
        .map(|a| a.key().ok_or("LIVE_ANCHOR_INVALID"))
        .transpose()?;
    let c = &view.connection;
    let table = username
        .map(|name| conversation_table(c, name))
        .transpose()?
        .unwrap_or_else(|| FILEHELPER_TABLE.to_owned());

    let found: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [&table],
            |r| r.get(0),
        )
        .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?;
    if !found {
        return Err("LIVE_CONVERSATION_NOT_IN_SOURCE");
    }
    let mut info = c
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
    let fields: Vec<(String, String, i64)> = info
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(5)?)))
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
    // Ordering must have an unambiguous per-database tie breaker.
    if fields.iter().filter(|(_, _, pk)| *pk != 0).count() != 1
        || !fields.iter().any(|(n, t, pk)| {
            n.eq_ignore_ascii_case("local_id") && t.eq_ignore_ascii_case("INTEGER") && *pk == 1
        })
    {
        return Err("LIVE_MESSAGE_KEY_UNSUPPORTED");
    }
    let columns: Vec<_> = fields.into_iter().map(|(n, _, _)| n).collect();
    let id = column(&columns, &["local_id"])?;
    let time = column(&columns, &["create_time", "createTime"])?;
    let kind = column(&columns, &["local_type"])?;
    let content = column(&columns, &["message_content", "messageContent", "content"])?;
    let server = column(&columns, &["server_id"]).unwrap_or_else(|_| "NULL".into());
    let compressed = column(&columns, &["compress_content"]).unwrap_or_else(|_| "NULL".into());
    let sender = sender_expression(c, &columns);
    let source = column(&columns, &["source", "msg_source", "message_source"]).unwrap_or_else(|_| "NULL".into());
    if let Some(anchor) = before {
        use rusqlite::OptionalExtension;
        let existing: Option<(i64, Option<i64>)> = c
            .query_row(
                &format!("SELECT {time},{server} FROM \"{table}\" WHERE {id}=?1"),
                [boundary.ok_or("LIVE_ANCHOR_INVALID")?.1],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?;
        if existing
            != Some((
                anchor.timestamp,
                anchor
                    .server_id
                    .as_ref()
                    .map(|s| s.parse::<i64>().expect("validated anchor")),
            ))
        {
            return Err("CURSOR_STALE");
        }
    }
    // Prefer nonempty compressed content lazily; never replace a corrupt
    // compressed body with another field that could be stale or unrelated.
    let selected = format!(
        "CASE WHEN coalesce(length(CAST({compressed} AS BLOB)),0)>0 THEN {compressed} ELSE {content} END"
    );
    let predicate = if let Some(filter) = filter {
        let filter = filter.clone();
        c.create_scalar_function(
            "werc_message_matches",
            3,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            move |context| {
                let kind: i64 = context.get(0)?;
                if (filter.text.is_none() && filter.app_types.is_empty())
                    || (kind != 1 && kind as u32 != 49)
                {
                    return Ok(filter.matches_content(kind, None));
                }
                let decoded = match context.get_raw(1) {
                    ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
                        crate::text::decode(bytes).ok()
                    }
                    _ => None,
                };
                let body = decoded.map(|(body, _)| zeroize::Zeroizing::new(body));
                let sender: Option<String> = context.get(2)?;
                Ok(filter.matches_body(
                    kind,
                    body.as_deref().map(String::as_str),
                    sender.as_deref(),
                ))
            },
        )
        .map_err(|_| "LIVE_QUERY_UNAVAILABLE")?;
        format!(
            " AND (?4 IS NULL OR {time}>=?4) AND (?5 IS NULL OR {time}<?5) AND (?6 IS NULL OR {server}=?6) AND (?7 IS NULL OR ({sender}) COLLATE BINARY=?7) AND werc_message_matches({kind},({selected}),({sender}))"
        )
    } else {
        // Keep the parameter contract constant for unfiltered legacy reads.
        " AND ?4 IS NULL AND ?5 IS NULL AND ?6 IS NULL AND ?7 IS NULL".into()
    };
    let comparator = if newer {
        ">"
    } else if include_anchor {
        "<="
    } else {
        "<"
    };
    let time_comparator = if newer { ">" } else { "<" };
    let order = if newer { "ASC" } else { "DESC" };
    let sql = format!(
        "SELECT {id},{time},{kind},{server},CASE WHEN ({kind}=1 OR ({kind} & 4294967295) IN (3,37,42,43,47,48,49,50,10000,10002)) AND length(CAST(({selected}) AS BLOB))<=65536 THEN ({selected}) ELSE NULL END,CASE WHEN ({kind}=1 OR ({kind} & 4294967295) IN (3,37,42,43,47,48,49,50,10000,10002)) THEN length(CAST(({selected}) AS BLOB)) ELSE NULL END,{sender},CASE WHEN length(CAST(({source}) AS BLOB))<=65536 THEN ({source}) ELSE NULL END FROM \"{table}\" AS m WHERE (?2 IS NULL OR {time}{time_comparator}?2 OR ({time}=?2 AND {id}{comparator}?3)){predicate} ORDER BY {time} {order},{id} {order} LIMIT ?1"
    );
    let mut statement = c
        .prepare(&sql)
        .map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
    let mut query = statement
        .query(rusqlite::params![
            limit as i64 + 1,
            boundary.map(|b| b.0),
            boundary.map(|b| b.1),
            filter.and_then(|q| q.from_timestamp),
            filter.and_then(|q| q.until_timestamp),
            filter
                .and_then(|q| q.server_id.as_ref())
                .and_then(|id| id.parse::<i64>().ok()),
            filter.and_then(|q| q.sender_username.as_deref())
        ])
        .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?;
    let mut rows = Vec::new();
    let mut budget = 512_usize;
    let mut previous = None;
    let mut has_older = false;
    while let Some(row) = query
        .next()
        .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?
    {
        view.check_deadline()?;
        if rows.len() == limit {
            has_older = true;
            break;
        }
        let local_id: i64 = row.get(0).map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
        let timestamp: i64 = row.get(1).map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
        let message_type: i64 = row.get(2).map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
        let server_id: Option<i64> = row.get(3).map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
        let content_length: Option<i64> =
            row.get(5).map_err(|_| "LIVE_MESSAGE_SCHEMA_UNSUPPORTED")?;
        if local_id <= 0
            || timestamp < 0
            || previous.is_some_and(|p| {
                if newer {
                    (timestamp, local_id) <= p
                } else {
                    (timestamp, local_id) >= p
                }
            })
        {
            return Err("LIVE_MESSAGE_ORDER_INVALID");
        }
        previous = Some((timestamp, local_id));
        let decoded = if message_type != 1
            && !matches!(message_type & 0xffffffff, 3 | 37 | 42 | 43 | 47 | 48 | 49 | 50 | 10000 | 10002)
        {
            Err("unsupported_type")
        } else if content_length.is_some_and(|n| n > crate::text::MAX_INPUT as i64) {
            Err("content_oversized")
        } else {
            match row
                .get_ref(4)
                .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?
            {
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => crate::text::decode(bytes),
                ValueRef::Null => Err("content_missing"),
                _ => Err("content_invalid"),
            }
        };
        let (text, metadata, content_status) = match decoded {
            Ok((metadata, status))
                if matches!(message_type & 0xffffffff, 3 | 37 | 42 | 43 | 47 | 48 | 49 | 50 | 10000 | 10002) =>
            {
                (
                    None,
                    Some(metadata),
                    if status == "text_zstd" {
                        "metadata_zstd"
                    } else {
                        "metadata"
                    }
                    .into(),
                )
            }
            Ok((text, status)) => (Some(text), None, status.into()),
            Err(status) => (None, None, status.into()),
        };
        let message = LiveMessage {
            at_user_list: match row.get_ref(7).map_err(|_| "LIVE_READ_FAILED")? {
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => crate::text::decode(bytes).ok().and_then(|(source, _)| {
                    let source = zeroize::Zeroizing::new(source);
                    parse_mention_source(&source)
                }),
                _ => None,
            },
            local_id: local_id.to_string(),
            server_id: server_id.map(|n| n.to_string()),
            timestamp,
            message_type: message_type.to_string(),
            text,
            content_status,
            metadata,
            sender_username: match row
                .get_ref(6)
                .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?
            {
                ValueRef::Text(bytes) => std::str::from_utf8(bytes)
                    .ok()
                    .filter(|s| valid_live_username(s))
                    .map(str::to_owned),
                _ => None,
            },
        };
        if filter.is_some_and(|q| !q.matches(&message)) {
            return Err("LIVE_QUERY_RESULT_INVALID");
        }
        budget += serde_json::to_vec(&message)
            .map_err(|_| view.check_deadline().err().unwrap_or("LIVE_READ_FAILED"))?
            .len()
            + 1;
        if budget > LIVE_WINDOW_MAX_BYTES {
            return Err("LIVE_WINDOW_BYTES_LIMIT");
        }
        rows.push(message);
        view.check_deadline()?;
    }
    drop(query);
    drop(statement);
    if newer {
        rows.reverse();
        has_older = false;
    }
    Ok(LiveWindow {
        schema_version: 1,
        rows,
        has_older,
        read_only: true,
        readonly_shm: true,
        read_mode: "bounded_transaction_window".into(),
    })
}

#[cfg(test)]
mod sender_tests {
    use super::*;
    #[test]
    fn missing_or_unsupported_mapping_schema_preserves_message_reads() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        let columns = vec!["real_sender_id".into()];
        assert_eq!(sender_expression(&c, &columns), "NULL");
        c.execute_batch("CREATE TABLE Name2Id(unrelated TEXT)")
            .unwrap();
        assert_eq!(sender_expression(&c, &columns), "NULL");
        c.execute_batch("DROP TABLE Name2Id; CREATE TABLE Name2Id(rowid INTEGER,user_name TEXT)")
            .unwrap();
        assert_eq!(sender_expression(&c, &columns), "NULL");
        c.execute_batch(
            "DROP TABLE Name2Id; CREATE TABLE Name2Id(user_name TEXT PRIMARY KEY) WITHOUT ROWID",
        )
        .unwrap();
        assert_eq!(sender_expression(&c, &columns), "NULL");
        c.execute_batch("DROP TABLE Name2Id; CREATE TABLE Name2Id(user_name TEXT)")
            .unwrap();
        assert_eq!(sender_expression(&c, &[]), "NULL");
        assert_ne!(sender_expression(&c, &columns), "NULL");
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;
    #[test]
    fn encrypted_source_query_filters_before_limit_and_reads_nearest_newer_rows() {
        crate::live::prepare_memory().unwrap();
        let root = std::env::temp_dir().join(format!(
            "werc-query-owned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("owned.sqlite3");
        let c = rusqlite::Connection::open(&path).unwrap();
        let key = "5a".repeat(32);
        c.execute_batch(&format!("PRAGMA key=\"x'{key}'\"; PRAGMA cipher_compatibility=4; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE \"{FILEHELPER_TABLE}\"(local_id INTEGER PRIMARY KEY,create_time INTEGER,local_type INTEGER,server_id INTEGER,message_content BLOB,real_sender_id INTEGER);CREATE TABLE Name2Id(user_name TEXT);INSERT INTO Name2Id VALUES('owned');")).unwrap();
        for n in 1..=450 {
            c.execute(&format!("INSERT INTO \"{FILEHELPER_TABLE}\" VALUES(?1,1700000000+?1,1,9007199254740992+?1,?2,1)"),rusqlite::params![n,if n==1 {"Find Me"}else{"ordinary"}]).unwrap();
        }
        let mut selection = LiveConversationSelection {
            username: "filehelper".into(),
            before: None,
            include_anchor: false,
            newer_context: false,
            query: Some(LiveMessageQuery::default()),
        };
        selection.query.as_mut().unwrap().text = Some("FIND ME".into());
        let result = read_selected(&path, &key, 2, Some(&selection), None).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].local_id, "1");
        assert!(!result.has_older);
        // Optional source metadata is independent of text and retains no tokens.
        assert!(result.rows[0].at_user_list.is_none());
        c.execute_batch(&format!("ALTER TABLE \"{FILEHELPER_TABLE}\" ADD COLUMN source BLOB")).unwrap();
        c.execute(&format!("UPDATE \"{FILEHELPER_TABLE}\" SET source=?1 WHERE local_id=1"),["<msgsource><atuserlist><![CDATA[owned,notify@all]]></atuserlist><ticket>private</ticket></msgsource>"]).unwrap();
        let mentions=read_selected(&path,&key,2,Some(&selection),None).unwrap();
        assert_eq!(mentions.rows[0].at_user_list,Some(vec!["owned".into(),"notify@all".into()]));
        assert!(!serde_json::to_string(&mentions).unwrap().contains("private"));
        c.execute(&format!("UPDATE \"{FILEHELPER_TABLE}\" SET source=?1 WHERE local_id=1"),["<msgsource><atuserlist>owned</atuserlist><atuserlist>other</atuserlist></msgsource>"]).unwrap();
        assert!(read_selected(&path,&key,2,Some(&selection),None).unwrap().rows[0].at_user_list.is_none());
        selection.query = None;
        selection.before = Some(LiveAnchor {
            timestamp: 1700000200,
            local_id: "200".into(),
            server_id: Some("9007199254741192".into()),
        });
        selection.newer_context = true;
        let result = read_selected(&path, &key, 2048, Some(&selection), None).unwrap();
        assert_eq!(result.rows.len(), 30);
        assert_eq!(result.rows[0].local_id, "230");
        assert_eq!(result.rows.last().unwrap().local_id, "201");
        assert!(!result.has_older);
        selection.newer_context = false;
        selection.include_anchor = true;
        let result = read_selected(&path, &key, 2, Some(&selection), None).unwrap();
        assert_eq!(result.rows[0].local_id, "200");
        assert_eq!(result.rows[1].local_id, "199");
        assert!(result.has_older);
        c.execute(&format!("UPDATE \"{FILEHELPER_TABLE}\" SET local_type=25769803825,message_content=?1 WHERE local_id=1"),["owned:\n<msg><appmsg><type>6</type></appmsg></msg>"]).unwrap();
        selection.before = None;
        selection.include_anchor = false;
        selection.query = Some(LiveMessageQuery::default());
        selection.query.as_mut().unwrap().app_types = vec![6];
        let result = read_selected(&path, &key, 2, Some(&selection), None).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].local_id, "1");
        // System/verification/call bodies reach the same strict display parser as
        // archived card bodies; they must not disappear or become raw text.
        selection.query.as_mut().unwrap().app_types.clear();
        for (kind, body) in [
            (37, "<msg fromusername='owned-peer' content='自动化测试申请' ticket='not-public'/>"),
            (50, "<voipmsg>已取消</voipmsg>"),
            (10000, "自动化测试：群成员变更通知"),
            (10002, "<sysmsg type='revokemsg'><revokemsg><newmsgid>9007199254740993</newmsgid><replacemsg>撤回通知</replacemsg></revokemsg></sysmsg>"),
        ] {
            c.execute(&format!("UPDATE \"{FILEHELPER_TABLE}\" SET local_type=?1,message_content=?2 WHERE local_id=1"),rusqlite::params![kind,body]).unwrap();
            selection.query.as_mut().unwrap().message_types=vec![kind as u32];
            let page=read_selected(&path,&key,2,Some(&selection),None).unwrap();
            assert_eq!(page.rows.len(),1);
            assert_eq!(page.rows[0].metadata.as_deref(),Some(body));
            assert_eq!(page.rows[0].text,None);
            assert_eq!(page.rows[0].content_status,"metadata");
        }
        drop(c);
        std::fs::remove_dir_all(root).unwrap();
    }
}
