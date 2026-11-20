//! Bounded pages from the observed local contact cache; never a server friend list.
use crate::{live::ReadView, live_protocol::*};
use rusqlite::{Connection, params};
use std::path::Path;
use zeroize::Zeroizing;

fn query_filtered(
    c: &Connection,
    after: Option<&LiveContactAnchor>,
    search_query: Option<&str>,
) -> Result<LiveContactWindow, &'static str> {
    if search_query.is_some_and(|q| !valid_contact_query(q)) {
        return Err("LIVE_QUERY_INVALID");
    }
    let needle = search_query.map(|q| Zeroizing::new(q.to_lowercase()));
    c.create_scalar_function(
        "werc_contact_matches",
        4,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        move |context| {
            let Some(needle) = needle.as_ref() else {
                return Ok(true);
            };
            let mut matched = false;
            for index in 0..4 {
                let value = match context.get_raw(index) {
                    rusqlite::types::ValueRef::Null if index != 0 => continue,
                    rusqlite::types::ValueRef::Text(bytes)
                        if bytes.len() <= if index == 0 { 256 } else { 1024 } =>
                    {
                        std::str::from_utf8(bytes).map_err(|_| rusqlite::Error::InvalidQuery)?
                    }
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                if value.contains('\0')
                    || (index == 0 && (value.is_empty() || value.chars().any(char::is_control)))
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                matched |= value.to_lowercase().contains(needle.as_str());
            }
            Ok(matched)
        },
    )
    .map_err(|_| "LIVE_QUERY_UNAVAILABLE")?;
    let after_id = after
        .map(|a| a.key().ok_or("LIVE_CONTACT_ANCHOR_INVALID"))
        .transpose()?
        .unwrap_or(0);
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='contact')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?;
    if !exists {
        return Err("LIVE_CONTACT_SCHEMA_UNSUPPORTED");
    }
    let columns: Vec<String> = c
        .prepare("SELECT name FROM pragma_table_info('contact') LIMIT 65")
        .map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?
        .query_map([], |r| r.get(0))
        .map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?
        .collect::<Result<_, _>>()
        .map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?;
    if columns.len() > 64
        || ![
            "username",
            "nick_name",
            "remark",
            "alias",
            "local_type",
            "delete_flag",
            "verify_flag",
        ]
        .iter()
        .all(|n| columns.iter().any(|c| c == n))
    {
        return Err("LIVE_CONTACT_SCHEMA_UNSUPPORTED");
    }
    if let Some(anchor) = after {
        let matches: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM contact WHERE rowid=?1 AND username=?2)",
                params![after_id, anchor.username],
                |r| r.get(0),
            )
            .map_err(|_| "LIVE_CONTACT_READ_FAILED")?;
        if !matches {
            return Err("LIVE_CONTACT_ANCHOR_CHANGED");
        }
    }
    // Check UTF-8 byte lengths inside SQLite before materializing strings.
    // Include one extra row only to determine whether another page exists.
    let avatar = if columns.iter().any(|c| c == "small_head_url") {
        "CASE WHEN typeof(small_head_url)='text' AND length(CAST(small_head_url AS BLOB))<=4096 THEN small_head_url END"
    } else {
        "NULL"
    };
    let description = if columns.iter().any(|c| c == "description") {
        "CASE WHEN typeof(description)='text' AND length(CAST(description AS BLOB))<=4096 THEN description END"
    } else {
        "NULL"
    };
    let flag = if columns.iter().any(|c| c == "flag") {
        "CASE WHEN typeof(flag)='integer' AND flag BETWEEN 0 AND 9007199254740991 THEN flag END"
    } else {
        "NULL"
    };
    // An absent/older optional business table must not hide the contact list.
    let business_columns = c
        .prepare("SELECT name FROM pragma_table_info('biz_info') LIMIT 65")
        .map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?;
    let business = if business_columns.len() <= 64
        && ["username", "type"]
            .iter()
            .all(|n| business_columns.iter().any(|c| c == n))
    {
        "(SELECT CASE WHEN count(DISTINCT type)=1 AND min(typeof(type))='integer' AND max(typeof(type))='integer' THEN min(type) END FROM biz_info WHERE biz_info.username=contact.username)"
    } else {
        "NULL"
    };
    let mut statement = c.prepare(&format!("SELECT rowid,
      CASE WHEN typeof(username)='text' AND length(CAST(username AS BLOB)) BETWEEN 1 AND 256 THEN username END,
      CASE WHEN nick_name IS NULL OR (typeof(nick_name)='text' AND length(CAST(nick_name AS BLOB))<=1024) THEN 1 ELSE 0 END,
      CASE WHEN typeof(nick_name)='text' AND length(CAST(nick_name AS BLOB))<=1024 THEN nick_name END,
      CASE WHEN remark IS NULL OR (typeof(remark)='text' AND length(CAST(remark AS BLOB))<=1024) THEN 1 ELSE 0 END,
      CASE WHEN typeof(remark)='text' AND length(CAST(remark AS BLOB))<=1024 THEN remark END,
      CASE WHEN alias IS NULL OR (typeof(alias)='text' AND length(CAST(alias AS BLOB))<=1024) THEN 1 ELSE 0 END,
      CASE WHEN typeof(alias)='text' AND length(CAST(alias AS BLOB))<=1024 THEN alias END,
      local_type,delete_flag,verify_flag,{avatar},{description},{flag},{business} FROM contact WHERE rowid>?1 AND werc_contact_matches(username,nick_name,remark,alias) ORDER BY rowid LIMIT ?2")).map_err(|_| "LIVE_CONTACT_SCHEMA_UNSUPPORTED")?;
    let mut rows = statement
        .query(params![after_id, LIVE_CONTACT_MAX_ROWS as i64 + 1])
        .map_err(|_| "LIVE_CONTACT_READ_FAILED")?;
    let mut result = LiveContactWindow {
        schema_version: 1,
        rows: Vec::new(),
        has_more: false,
        read_only: true,
        readonly_shm: true,
    };
    while let Some(row) = rows.next().map_err(|_| "LIVE_CONTACT_READ_FAILED")? {
        if result.rows.len() == LIVE_CONTACT_MAX_ROWS {
            result.has_more = true;
            break;
        }
        for index in [2, 4, 6] {
            if !row
                .get::<_, bool>(index)
                .map_err(|_| "LIVE_CONTACT_INVALID")?
            {
                return Err("LIVE_CONTACT_FIELD_LIMIT");
            }
        }
        let item = LiveContact {
            anchor: LiveContactAnchor {
                row_id: row
                    .get::<_, i64>(0)
                    .map_err(|_| "LIVE_CONTACT_INVALID")?
                    .to_string(),
                username: row.get(1).map_err(|_| "LIVE_CONTACT_INVALID")?,
            },
            nickname: row.get(3).map_err(|_| "LIVE_CONTACT_INVALID")?,
            remark: row.get(5).map_err(|_| "LIVE_CONTACT_INVALID")?,
            alias: row.get(7).map_err(|_| "LIVE_CONTACT_INVALID")?,
            local_type: row.get(8).map_err(|_| "LIVE_CONTACT_INVALID")?,
            delete_flag: row.get(9).map_err(|_| "LIVE_CONTACT_INVALID")?,
            verify_flag: row.get(10).map_err(|_| "LIVE_CONTACT_INVALID")?,
            contact_flag: row.get(13).map_err(|_| "LIVE_CONTACT_INVALID")?,
            business_type: row.get(14).map_err(|_| "LIVE_CONTACT_INVALID")?,
            avatar_url: row
                .get::<_, Option<String>>(11)
                .map_err(|_| "LIVE_CONTACT_INVALID")?
                .as_deref()
                .and_then(contact_avatar_url),
            description: row
                .get::<_, Option<String>>(12)
                .map_err(|_| "LIVE_CONTACT_INVALID")?
                .filter(|s| !s.contains('\0')),
        };
        if !item.valid() {
            return Err("LIVE_CONTACT_INVALID");
        }
        result.rows.push(item);
    }
    if !result.valid(after) {
        return Err("LIVE_CONTACT_INVALID");
    }
    let bytes = Zeroizing::new(serde_json::to_vec(&result).map_err(|_| "LIVE_CONTACT_INVALID")?);
    if bytes.len() > LIVE_CONTACT_MAX_BYTES {
        return Err("LIVE_CONTACT_WINDOW_LIMIT");
    }
    Ok(result)
}

pub(crate) fn read_filtered(
    path: &Path,
    key: &str,
    after: Option<&LiveContactAnchor>,
    search_query: Option<&str>,
) -> Result<LiveContactWindow, &'static str> {
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
    after: Option<&LiveContactAnchor>,
) -> Result<LiveContactWindow, &'static str> {
    query_filtered(c, after, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relationship_fields_are_optional_and_conflicting_business_types_stay_unknown() {
        let c = fixture();
        insert(&c, 1, "gh_owned", None);
        let row = query(&c, None).unwrap().rows.remove(0);
        assert!(row.contact_flag.is_none() && row.business_type.is_none());
        c.execute_batch("ALTER TABLE contact ADD flag INTEGER; UPDATE contact SET flag=8; CREATE TABLE biz_info(username TEXT,type INTEGER); INSERT INTO biz_info VALUES('gh_owned',1),('gh_owned',1);").unwrap();
        let page = query(&c, None).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0].contact_flag, Some(8));
        assert_eq!(page.rows[0].business_type, Some(1));
        c.execute_batch("INSERT INTO biz_info VALUES('gh_owned',0); UPDATE contact SET flag=-1;")
            .unwrap();
        let page = query(&c, None).unwrap();
        assert!(page.rows[0].business_type.is_none());
        assert!(page.rows[0].contact_flag.is_none());
        c.execute_batch(
            "DROP TABLE biz_info; CREATE TABLE biz_info(username TEXT,unknown INTEGER);",
        )
        .unwrap();
        assert!(query(&c, None).unwrap().rows[0].business_type.is_none());
    }
    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE contact(username TEXT,nick_name TEXT,remark TEXT,alias TEXT,local_type INTEGER,delete_flag INTEGER,verify_flag INTEGER)").unwrap();
        c
    }
    #[test]
    fn optional_avatar_and_description_keep_old_layout_compatible() {
        let c = fixture();
        insert(&c, 1, "owned", None);
        assert!(query(&c, None).unwrap().rows[0].avatar_url.is_none());
        c.execute_batch("ALTER TABLE contact ADD small_head_url TEXT; ALTER TABLE contact ADD description TEXT;").unwrap();
        c.execute(
            "UPDATE contact SET small_head_url=?1,description='自有简介'",
            ["http://wx.qlogo.cn/owned/132"],
        )
        .unwrap();
        let page = query(&c, None).unwrap();
        assert_eq!(
            page.rows[0].avatar_url.as_deref(),
            Some("https://wx.qlogo.cn/owned/132")
        );
        assert_eq!(page.rows[0].description.as_deref(), Some("自有简介"));
        for value in [
            "https://qlogo.cn.evil.test/a",
            "https://wx.qlogo.cn@evil.test/a",
            "https://wx.qlogo.cn:80/a",
            "file:///owned",
            "https://evil.test\\@wx.qlogo.cn/a",
        ] {
            c.execute("UPDATE contact SET small_head_url=?1", [value])
                .unwrap();
            assert!(query(&c, None).unwrap().rows[0].avatar_url.is_none());
        }
    }
    fn insert(c: &Connection, id: i64, name: &str, remark: Option<&str>) {
        c.execute("INSERT INTO contact(rowid,username,nick_name,remark,alias,local_type,delete_flag,verify_flag) VALUES(?1,?2,'自有昵称',?3,NULL,0,0,0)", params![id,name,remark]).unwrap();
    }
    #[test]
    fn search_reaches_beyond_first_page_and_matches_remark() {
        let c = fixture();
        for n in 1..=401 {
            insert(
                &c,
                n,
                &format!("owned_{n}"),
                Some(if n == 401 { "特殊备注" } else { "普通" }),
            );
        }
        let found = query_filtered(&c, None, Some("特殊")).unwrap();
        assert_eq!(found.rows.len(), 1);
        assert_eq!(found.rows[0].anchor.username, "owned_401");
        assert!(!found.has_more);
        let first = query_filtered(&c, None, Some("OWNED_")).unwrap();
        assert_eq!(first.rows.len(), 200);
        assert!(first.has_more);
        let next =
            query_filtered(&c, first.rows.last().map(|r| &r.anchor), Some("OWNED_")).unwrap();
        assert_eq!(next.rows[0].anchor.username, "owned_201");
    }
    #[test]
    fn exact_pages_preserve_unicode_and_large_ids_and_reject_changed_boundary() {
        let c = fixture();
        for n in 1..=201 {
            insert(
                &c,
                9007199254740992 + n,
                &format!("owned_{n}"),
                Some("备注 <> &"),
            );
        }
        let first = query(&c, None).unwrap();
        assert_eq!(first.rows.len(), 200);
        assert!(first.has_more);
        assert_eq!(first.rows[0].anchor.row_id, "9007199254740993");
        assert_eq!(first.rows[0].remark.as_deref(), Some("备注 <> &"));
        let anchor = &first.rows.last().unwrap().anchor;
        let last = query(&c, Some(anchor)).unwrap();
        assert_eq!(last.rows.len(), 1);
        assert!(!last.has_more);
        c.execute(
            "DELETE FROM contact WHERE rowid=?1",
            [anchor.key().unwrap()],
        )
        .unwrap();
        assert!(matches!(
            query(&c, Some(anchor)),
            Err("LIVE_CONTACT_ANCHOR_CHANGED")
        ));
    }
    #[test]
    fn rejects_oversize_invalid_fields_and_wrong_schema_without_partial_results() {
        for value in ["x".repeat(1025), "中".repeat(342), "bad\0text".into()] {
            let c = fixture();
            insert(&c, 1, "owned", Some(&value));
            assert!(query(&c, None).is_err());
        }
        for name in ["", "a\0b", "a\nb"] {
            let c = fixture();
            insert(&c, 1, name, None);
            assert!(query(&c, None).is_err());
        }
        let c = fixture();
        insert(&c, 1, "owned", None);
        c.execute("UPDATE contact SET remark=x'fffe'", []).unwrap();
        assert!(query(&c, None).is_err());
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE VIEW contact AS SELECT 'owned' username")
            .unwrap();
        assert!(matches!(
            query(&c, None),
            Err("LIVE_CONTACT_SCHEMA_UNSUPPORTED")
        ));
    }
}
