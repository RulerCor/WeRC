//! Query the observed favorite.db table, without WeChat's custom FTS tokenizer.
use crate::{favorites_protocol::*, live::ReadView};
use rusqlite::{Connection, params};
use std::path::Path;

// Profile 4.1.15.13: fav_bind_tag_db_item.op_code is NONE=0, ADD=1,
// DEL=2 (native enum formatter 0x2f12cc0). XML taglist is not authoritative
// once these relationship tables exist.
fn has_tag_tables(c: &Connection) -> Result<bool, &'static str> {
    let count: u32 = c.query_row("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN ('fav_tag_db_item','fav_bind_tag_db_item')", [], |r|r.get(0))
        .map_err(|_| "FAVORITE_SCHEMA_UNSUPPORTED")?;
    match count { 0 => Ok(false), 2 => Ok(true), _ => Err("FAVORITE_TAG_SCHEMA_UNSUPPORTED") }
}
fn tags(c: &Connection, local: i64, server: i64) -> Result<Vec<String>, &'static str> {
    let mut q = c.prepare("SELECT t.name,min(b.op_code),max(b.op_code) FROM fav_bind_tag_db_item b
        JOIN fav_tag_db_item t ON (b.tag_local_id>0 AND t.local_id=b.tag_local_id)
          OR (coalesce(b.tag_local_id,0)=0 AND b.tag_server_id!=0 AND t.server_id=b.tag_server_id)
        WHERE (b.fav_local_id=?1 OR (coalesce(b.fav_local_id,0)=0 AND ?2!=0 AND b.fav_server_id=?2))
          AND b.op_code!=2 GROUP BY t.name ORDER BY t.name LIMIT 129").map_err(|_|"FAVORITE_TAG_SCHEMA_UNSUPPORTED")?;
    let mut rows=q.query(params![local,server]).map_err(|_|"FAVORITE_READ_FAILED")?;
    let mut result=Vec::new();
    while let Some(row)=rows.next().map_err(|_|"FAVORITE_READ_FAILED")? {
        let op:i64=row.get(1).map_err(|_|"FAVORITE_TAG_SCHEMA_UNSUPPORTED")?;
        let last_op:i64=row.get(2).map_err(|_|"FAVORITE_TAG_SCHEMA_UNSUPPORTED")?;
        if !matches!(op,0|1) || !matches!(last_op,0|1) {return Err("FAVORITE_TAG_SCHEMA_UNSUPPORTED")}
        let value=row.get_ref(0).map_err(|_|"FAVORITE_READ_FAILED")?.as_str().map_err(|_|"FAVORITE_TAG_SCHEMA_UNSUPPORTED")?;
        if value.is_empty() || value.len()>256 || value.chars().any(char::is_control) {return Err("FAVORITE_TAG_SCHEMA_UNSUPPORTED")}
        if result.last().is_none_or(|v|v!=value) {result.push(value.to_owned());}
        if result.len()>128 {return Err("FAVORITE_TAG_LIMIT")}
    }
    Ok(result)
}

fn query(c: &Connection, s: &FavoriteSelection) -> Result<FavoriteWindow, &'static str> {
    if !s.valid() {
        return Err("FAVORITE_SELECTION_INVALID");
    }
    let with_tags = has_tag_tables(c)?;
    let mut result = FavoriteWindow {
        rows: vec![],
        next: None,
        counts: Default::default(),
        scanned: 0,
        read_only: true,
        readonly_shm: true,
    };
    let mut counts = c
        .prepare("SELECT type,count(*) FROM fav_db_item GROUP BY type LIMIT 129")
        .map_err(|_| "FAVORITE_SCHEMA_UNSUPPORTED")?;
    for item in counts
        .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u64>(1)?)))
        .map_err(|_| "FAVORITE_READ_FAILED")?
    {
        let (kind, n) = item.map_err(|_| "FAVORITE_READ_FAILED")?;
        result.counts.insert(kind, n);
    }
    if result.counts.len() > 128 {
        return Err("FAVORITE_SCHEMA_UNSUPPORTED");
    }
    let boundary = s.after.as_ref().and_then(FavoriteAnchor::key);
    if let Some((time, id)) = boundary {
        let exists: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM fav_db_item WHERE local_id=?1 AND update_time=?2)",
                params![id, time],
                |r| r.get(0),
            )
            .map_err(|_| "FAVORITE_READ_FAILED")?;
        if !exists {
            return Err("FAVORITE_ANCHOR_CHANGED");
        }
    }
    let sql="SELECT local_id,server_id,type,update_time,
        CASE WHEN typeof(fromusr)='text' AND length(CAST(fromusr AS BLOB))<=256 THEN fromusr END,
        CASE WHEN typeof(realchatname)='text' AND length(CAST(realchatname AS BLOB))<=256 THEN realchatname END,
        CASE WHEN typeof(content)='text' AND length(CAST(content AS BLOB))<=262144 THEN content END,
        CASE WHEN content IS NULL THEN 'missing' WHEN typeof(content)!='text' THEN 'unsupported' WHEN length(CAST(content AS BLOB))>262144 THEN 'oversized' ELSE 'parsed' END
        FROM fav_db_item WHERE (?1 IS NULL OR type=?1) AND (?2 IS NULL OR update_time>=?2) AND (?3 IS NULL OR update_time<?3)
        AND (?4 IS NULL OR (?6=1 AND update_time=?4 AND local_id=?5) OR (?6=0 AND (update_time<?4 OR (update_time=?4 AND local_id<?5))))
        ORDER BY update_time DESC,local_id DESC LIMIT 201";
    let mut q = c.prepare(sql).map_err(|_| "FAVORITE_SCHEMA_UNSUPPORTED")?;
    let mut rows = q
        .query(params![
            s.query.kind,
            s.query.from,
            s.query.until,
            boundary.map(|v| v.0),
            boundary.map(|v| v.1),
            s.exact
        ])
        .map_err(|_| "FAVORITE_READ_FAILED")?;
    let needle = s.query.text.to_lowercase();
    let mut last = None;
    let mut bytes = 0;
    while let Some(row) = rows.next().map_err(|_| "FAVORITE_READ_FAILED")? {
        if result.rows.len() >= MAX_ROWS || result.scanned >= 200 {
            result.next = last;
            break;
        }
        let mut item = FavoriteRaw {
            anchor: FavoriteAnchor {
                local_id: row
                    .get::<_, i64>(0)
                    .map_err(|_| "FAVORITE_READ_FAILED")?
                    .to_string(),
                updated: row.get(3).map_err(|_| "FAVORITE_READ_FAILED")?,
            },
            server_id: (row.get::<_, i64>(1).map_err(|_| "FAVORITE_READ_FAILED")? as u64)
                .to_string(),
            kind: row.get(2).map_err(|_| "FAVORITE_READ_FAILED")?,
            source_user: row.get(4).map_err(|_| "FAVORITE_READ_FAILED")?,
            conversation: row.get(5).map_err(|_| "FAVORITE_READ_FAILED")?,
            xml: row.get(6).map_err(|_| "FAVORITE_READ_FAILED")?,
            content_status: row.get(7).map_err(|_| "FAVORITE_READ_FAILED")?,
            tags: if with_tags { Some(tags(c,
                row.get(0).map_err(|_|"FAVORITE_READ_FAILED")?,
                row.get(1).map_err(|_|"FAVORITE_READ_FAILED")?)?) } else {None},
        };
        let row_bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(&item).map_err(|_| "FAVORITE_PROTOCOL_INVALID")?,
        )
        .len();
        if bytes + row_bytes > MAX_BYTES - 16384 {
            result.next = last;
            break;
        }
        result.scanned += 1;
        last = Some(item.anchor.clone());
        let content = item.display_content();
        if item.xml.is_some() && content.is_none() {
            item.content_status = "invalid".into()
        }
        if !needle.is_empty() || s.query.tag.is_some() {
            let Some(content) = content else { continue };
            if s.query
                .tag
                .as_ref()
                .is_some_and(|tag| !content.tags.contains(tag))
            {
                continue;
            }
            let display = format!(
                "{}\n{}\n{}\n{}",
                content.title,
                content.text,
                content.tags.join("\n"),
                content
                    .blocks
                    .iter()
                    .map(|v| format!("{} {}", v.title, v.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            if !display.to_lowercase().contains(&needle) {
                continue;
            }
        }
        bytes += row_bytes + 1;
        result.rows.push(item);
        if s.exact {
            break;
        }
    }
    if !result.valid(s) {
        return Err("FAVORITE_PROTOCOL_INVALID");
    }
    let encoded = zeroize::Zeroizing::new(
        serde_json::to_vec(&result).map_err(|_| "FAVORITE_PROTOCOL_INVALID")?,
    );
    if encoded.len() > MAX_BYTES {
        return Err("FAVORITE_WINDOW_LIMIT");
    }
    Ok(result)
}
pub(crate) fn read(
    path: &Path,
    key: &str,
    selection: &FavoriteSelection,
) -> Result<FavoriteWindow, &'static str> {
    let view = ReadView::open(path, key)?;
    let result = query(&view.connection, selection)?;
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
    fn relationship_tags_override_stale_xml_and_match_exact_ids() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE fav_db_item(local_id INTEGER,server_id INTEGER,type INTEGER,update_time INTEGER,fromusr TEXT,realchatname TEXT,content TEXT);
            CREATE TABLE fav_tag_db_item(local_id INTEGER,server_id INTEGER,name TEXT,seq INTEGER);
            CREATE TABLE fav_bind_tag_db_item(tag_local_id INTEGER,tag_server_id INTEGER,fav_local_id INTEGER,fav_server_id INTEGER,op_code INTEGER);
            INSERT INTO fav_db_item VALUES(1,-2,1,1700000000,'owned','filehelper','<favitem type=\"1\"><desc>自动化测试</desc><taglist><tag>已移除</tag></taglist></favitem>');
            INSERT INTO fav_tag_db_item VALUES(10,-10,'已同步',1),(20,-20,'待同步',1),(30,-30,'已移除',1),(40,-40,'服务器关联',1),(50,-50,'其他记录',1);
            INSERT INTO fav_bind_tag_db_item VALUES(10,-10,1,-2,0),(20,-20,1,-2,1),(30,-30,1,-2,2),(0,-40,0,-2,0),(50,-50,99,-2,0);").unwrap();
        let mut s=FavoriteSelection {query: FavoriteQuery::default(),after:None,exact:false};
        let page=query(&c,&s).unwrap();
        let content=page.rows[0].display_content().unwrap();
        assert_eq!(content.tags.len(),3);
        for value in ["已同步","待同步","服务器关联"] {assert!(content.tags.iter().any(|v|v==value));}
        for value in ["已移除","其他记录"] {
            s.query.tag=Some(value.into());assert!(query(&c,&s).unwrap().rows.is_empty());
        }
        s.query.tag=Some("服务器关联".into());
        assert_eq!(query(&c,&s).unwrap().rows.len(),1);
        c.execute("DELETE FROM fav_bind_tag_db_item",[]).unwrap();
        s.query.tag=None;
        assert!(query(&c,&s).unwrap().rows[0].display_content().unwrap().tags.is_empty());
        c.execute("INSERT INTO fav_bind_tag_db_item VALUES(10,-10,1,-2,99)",[]).unwrap();
        assert!(matches!(query(&c,&s),Err("FAVORITE_TAG_SCHEMA_UNSUPPORTED")));
    }
    #[test]
    fn favorite_pages_search_display_fields_and_exact_identity() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE fav_db_item(local_id INTEGER,server_id INTEGER,type INTEGER,update_time INTEGER,fromusr TEXT,realchatname TEXT,content TEXT)").unwrap();
        for n in 1..=40 {
            c.execute("INSERT INTO fav_db_item VALUES(?1,?2,1,1700000000,'owned','filehelper',?3)",params![n,i64::MAX,"<favitem type=\"1\"><desc>自动化测试</desc><cdn_datakey>secret-value</cdn_datakey><taglist><tag>测试</tag></taglist></favitem>"]).unwrap();
        }
        let mut s = FavoriteSelection {
            query: FavoriteQuery::default(),
            after: None,
            exact: false,
        };
        let first = query(&c, &s).unwrap();
        assert_eq!(first.rows.len(), 30);
        assert!(first.next.is_some());
        s.after = first.next;
        assert_eq!(query(&c, &s).unwrap().rows.len(), 10);
        s.exact = true;
        assert_eq!(query(&c, &s).unwrap().rows[0].anchor.local_id, "11");
        s.after = None;
        s.exact = false;
        s.query.text = "secret-value".into();
        assert!(query(&c, &s).unwrap().rows.is_empty());
        s.query.text = "自动化测试".into();
        s.query.tag = Some("测试".into());
        assert_eq!(query(&c, &s).unwrap().rows.len(), 30);
    }
}
