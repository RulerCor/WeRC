//! Same-account contact cache. One short read-only transaction per bounded page.
use crate::{group_protocol::*, live::ReadView};
use rusqlite::{Connection, Row, params, types::ValueRef};
use std::path::Path;
#[path = "group_names.rs"]
mod names;
#[path = "group_pins.rs"]
mod pins;
type Result<T> = std::result::Result<T, &'static str>;
fn columns(c: &Connection, table: &str, required: &[&str], rowid: bool) -> bool {
    let Ok(mut q) = c.prepare("SELECT name FROM pragma_table_info(?1) LIMIT 65") else {
        return false;
    };
    let Ok(rows) = q.query_map([table], |r| r.get::<_, String>(0)) else {
        return false;
    };
    let Ok(fields) = rows.collect::<rusqlite::Result<Vec<_>>>() else {
        return false;
    };
    fields.len()<=64 && required.iter().all(|n|fields.iter().any(|f|f.eq_ignore_ascii_case(n)))
        && (!rowid || !fields.iter().any(|f|["rowid","_rowid_","oid"].iter().any(|n|f.eq_ignore_ascii_case(n))))
        && c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1 COLLATE NOCASE)",[table],|r|r.get::<_,bool>(0)).unwrap_or(false)
        && (!rowid || c.prepare(&format!("SELECT rowid FROM {table} LIMIT 0")).is_ok())
}
fn text(row: &Row<'_>, index: usize, identity: bool) -> Result<Option<String>> {
    let text = match row.get_ref(index).map_err(|_| "LIVE_GROUP_READ_FAILED")? {
        ValueRef::Text(bytes) => std::str::from_utf8(bytes).ok(),
        _ => None,
    };
    Ok(text
        .filter(|s| {
            if identity {
                !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
            } else {
                s.len() <= 1024 && !s.contains('\0')
            }
        })
        .map(str::to_owned))
}
fn member(row: &Row<'_>) -> Result<LiveGroupMember> {
    let username = text(row, 2, true)?;
    let known = username.is_some();
    let value = LiveGroupMember {
        group_nickname: None,
        anchor: LiveGroupAnchor {
            row_id: row
                .get::<_, i64>(0)
                .map_err(|_| "LIVE_GROUP_INVALID")?
                .to_string(),
            member_id: row
                .get::<_, Option<i64>>(1)
                .map_err(|_| "LIVE_GROUP_INVALID")?
                .map(|n| n.to_string()),
            username,
        },
        nickname: if known { text(row, 3, false)? } else { None },
        remark: if known { text(row, 4, false)? } else { None },
    };
    if !value.valid() {
        return Err("LIVE_GROUP_INVALID");
    }
    Ok(value)
}
fn query(c: &Connection, selection: &LiveGroupSelection) -> Result<LiveGroupWindow> {
    if !selection.valid() {
        return Err("LIVE_GROUP_SELECTION_INVALID");
    }
    if !columns(c, "chat_room", &["id", "username"], false)
        || !columns(c, "name2id", &["username"], true)
        || !columns(c, "chatroom_member", &["room_id", "member_id"], true)
    {
        return Err("LIVE_GROUP_SCHEMA_UNSUPPORTED");
    }
    let owner = if columns(c, "chat_room", &["owner"], false) {
        "CASE WHEN typeof(r.owner)='text' AND length(CAST(r.owner AS BLOB)) BETWEEN 1 AND 256 THEN r.owner END"
    } else {
        "NULL"
    };
    let buffer = if columns(c, "chat_room", &["ext_buffer"], false) {
        "CASE WHEN typeof(r.ext_buffer)='blob' AND length(r.ext_buffer)<=2097152 THEN r.ext_buffer END"
    } else {
        "NULL"
    };
    let mut room_query=c.prepare(&format!("SELECT r.id,n.rowid,{owner},{buffer} FROM chat_room r LEFT JOIN name2id n ON n.username=r.username COLLATE BINARY WHERE r.username=?1 COLLATE BINARY LIMIT 2")).map_err(|_|"LIVE_GROUP_SCHEMA_UNSUPPORTED")?;
    let mut room_rows = room_query
        .query([&selection.username])
        .map_err(|_| "LIVE_GROUP_READ_FAILED")?;
    let row = room_rows
        .next()
        .map_err(|_| "LIVE_GROUP_READ_FAILED")?
        .ok_or("LIVE_GROUP_NOT_IN_CACHE")?;
    let room_id: i64 = row.get(0).map_err(|_| "LIVE_GROUP_INVALID")?;
    let name_id: Option<i64> = row.get(1).map_err(|_| "LIVE_GROUP_INVALID")?;
    let owner_username = text(row, 2, true)?;
    let buffer = zeroize::Zeroizing::new(
        row.get::<_, Option<Vec<u8>>>(3)
            .map_err(|_| "LIVE_GROUP_INVALID")?
            .unwrap_or_default(),
    );
    let group_names = names::decode(&buffer).unwrap_or_default();
    if room_id <= 0
        || name_id != Some(room_id)
        || room_rows
            .next()
            .map_err(|_| "LIVE_GROUP_READ_FAILED")?
            .is_some()
    {
        return Err("LIVE_GROUP_IDENTITY_UNVERIFIED");
    }
    let cached_pins=read_pins(c, &selection.username, room_id)?;
    let enrich = columns(
        c,
        "contact",
        &["id", "username", "nick_name", "remark"],
        false,
    );
    let join = if enrich {
        "LEFT JOIN contact c ON c.id=m.member_id AND c.username=n.username COLLATE BINARY"
    } else {
        ""
    };
    let nickname = if enrich {
        "CASE WHEN typeof(c.nick_name)='text' AND length(CAST(c.nick_name AS BLOB))<=1024 THEN c.nick_name END"
    } else {
        "NULL"
    };
    let remark = if enrich {
        "CASE WHEN typeof(c.remark)='text' AND length(CAST(c.remark AS BLOB))<=1024 THEN c.remark END"
    } else {
        "NULL"
    };
    let select = format!(
        "SELECT m.rowid,m.member_id,CASE WHEN typeof(n.username)='text' AND length(CAST(n.username AS BLOB)) BETWEEN 1 AND 256 THEN n.username END,{nickname},{remark} FROM chatroom_member m LEFT JOIN name2id n ON typeof(m.member_id)='integer' AND m.member_id>0 AND n.rowid=m.member_id {join} WHERE m.room_id=?1"
    );
    if let Some(after) = &selection.after {
        let mut q = c
            .prepare(&format!("{select} AND m.rowid=?2 LIMIT 2"))
            .map_err(|_| "LIVE_GROUP_SCHEMA_UNSUPPORTED")?;
        let mut rows = q
            .query(params![
                room_id,
                after.key().ok_or("LIVE_GROUP_SELECTION_INVALID")?
            ])
            .map_err(|_| "LIVE_GROUP_READ_FAILED")?;
        let current = member(
            rows.next()
                .map_err(|_| "LIVE_GROUP_READ_FAILED")?
                .ok_or("LIVE_GROUP_ANCHOR_CHANGED")?,
        )?;
        if current.anchor != *after || rows.next().map_err(|_| "LIVE_GROUP_READ_FAILED")?.is_some()
        {
            return Err("LIVE_GROUP_ANCHOR_CHANGED");
        }
    }
    let mut q = c
        .prepare(&format!(
            "{select} AND m.rowid>?2 ORDER BY m.rowid LIMIT ?3"
        ))
        .map_err(|_| "LIVE_GROUP_SCHEMA_UNSUPPORTED")?;
    let mut rows = q
        .query(params![
            room_id,
            selection
                .after
                .as_ref()
                .and_then(LiveGroupAnchor::key)
                .unwrap_or(0),
            LIVE_GROUP_MAX_ROWS as i64 + 1
        ])
        .map_err(|_| "LIVE_GROUP_READ_FAILED")?;
    let mut result = LiveGroupWindow {
        schema_version: 1,
        username: selection.username.clone(),
        owner_username,
        cached_pins,
        rows: Vec::new(),
        has_more: false,
        read_only: true,
        readonly_shm: true,
    };
    while let Some(row) = rows.next().map_err(|_| "LIVE_GROUP_READ_FAILED")? {
        if result.rows.len() == LIVE_GROUP_MAX_ROWS {
            result.has_more = true;
            break;
        }
        let mut value = member(row)?;
        value.group_nickname = value
            .anchor
            .username
            .as_deref()
            .and_then(|id| group_names.get(id))
            .cloned();
        result.rows.push(value);
    }
    if !result.valid(selection) {
        return Err("LIVE_GROUP_INVALID");
    }
    let bytes =
        zeroize::Zeroizing::new(serde_json::to_vec(&result).map_err(|_| "LIVE_GROUP_INVALID")?);
    if bytes.len() > LIVE_GROUP_MAX_BYTES {
        return Err("LIVE_GROUP_WINDOW_LIMIT");
    }
    Ok(result)
}
fn read_pins(c:&Connection, username:&str, room_id:i64) -> Result<Option<Vec<LiveGroupPin>>> {
    if !columns(c,"chat_room_info_detail", &["room_id_","username_","ext_buffer_"],false) {return Ok(None);}
    let mut q=c.prepare("SELECT room_id_,CASE WHEN typeof(ext_buffer_)='blob' AND length(ext_buffer_)<=2097152 THEN ext_buffer_ END FROM chat_room_info_detail WHERE username_=?1 COLLATE BINARY LIMIT 2").map_err(|_|"LIVE_GROUP_READ_FAILED")?;
    let mut rows=q.query([username]).map_err(|_|"LIVE_GROUP_READ_FAILED")?;
    let Some(row)=rows.next().map_err(|_|"LIVE_GROUP_READ_FAILED")? else {return Ok(None);};
    let id=row.get::<_,i64>(0).ok();
    let buffer=zeroize::Zeroizing::new(row.get::<_,Option<Vec<u8>>>(1).map_err(|_|"LIVE_GROUP_READ_FAILED")?);
    if id!=Some(room_id) || rows.next().map_err(|_|"LIVE_GROUP_READ_FAILED")?.is_some() {return Ok(None);}
    Ok(buffer.as_deref().and_then(pins::decode))
}
pub(crate) fn read(
    path: &Path,
    key: &str,
    selection: &LiveGroupSelection,
) -> Result<LiveGroupWindow> {
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
    fn cached_pins_require_exact_room_identity_and_distinguish_absent_and_empty() {
        let c=fixture();let s=LiveGroupSelection{username:"owned@chatroom".into(),after:None};
        assert!(query(&c,&s).unwrap().cached_pins.is_none());
        c.execute_batch("CREATE TABLE chat_room_info_detail(room_id_ INTEGER,username_ TEXT,ext_buffer_ BLOB); INSERT INTO chat_room_info_detail VALUES(1,'owned@chatroom',X'0A040A02182A'),(2,'other@chatroom',X'0A040A021829')").unwrap();
        assert_eq!(query(&c,&s).unwrap().cached_pins.as_ref().unwrap()[0].server_id,"42");
        c.execute_batch("UPDATE chat_room_info_detail SET ext_buffer_=X'0A00' WHERE room_id_=1").unwrap();
        assert!(query(&c,&s).unwrap().cached_pins.as_ref().unwrap().is_empty());
        c.execute_batch("UPDATE chat_room_info_detail SET room_id_=3 WHERE room_id_=1").unwrap();
        assert!(query(&c,&s).unwrap().cached_pins.is_none());
        c.execute_batch("UPDATE chat_room_info_detail SET room_id_=1 WHERE room_id_=3; INSERT INTO chat_room_info_detail VALUES(1,'owned@chatroom',X'0A00')").unwrap();
        assert!(query(&c,&s).unwrap().cached_pins.is_none());
    }
    #[test]
    fn optional_room_names_match_exact_identity_and_leave_contact_name_intact() {
        let c = fixture();
        c.execute_batch("ALTER TABLE chat_room ADD ext_buffer BLOB")
            .unwrap();
        fn field(number: u8, value: &[u8]) -> Vec<u8> {
            let mut out = vec![(number << 3) | 2, value.len() as u8];
            out.extend(value);
            out
        }
        let mut member = field(1, b"owned-member-1");
        member.extend(field(2, "群内称呼".as_bytes()));
        let bytes = field(1, &member);
        c.execute("UPDATE chat_room SET ext_buffer=?1 WHERE id=1", [bytes])
            .unwrap();
        let selection = LiveGroupSelection {
            username: "owned@chatroom".into(),
            after: None,
        };
        let page = query(&c, &selection).unwrap();
        assert_eq!(page.rows[0].group_nickname.as_deref(), Some("群内称呼"));
        assert_eq!(page.rows[0].nickname.as_deref(), Some("昵称"));
        assert!(page.rows[1].group_nickname.is_none());
        c.execute(
            "UPDATE chat_room SET ext_buffer=?1 WHERE id=1",
            [vec![255u8; 12]],
        )
        .unwrap();
        assert!(
            query(&c, &selection)
                .unwrap()
                .rows
                .iter()
                .all(|r| r.group_nickname.is_none())
        );
    }
    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE chat_room(id INTEGER,username TEXT,owner TEXT); CREATE TABLE name2id(username TEXT); CREATE TABLE chatroom_member(room_id INTEGER,member_id INTEGER); CREATE TABLE contact(id INTEGER,username TEXT,nick_name TEXT,remark TEXT); INSERT INTO chat_room VALUES(1,'owned@chatroom','owned-member-1'),(2,'other@chatroom',NULL); INSERT INTO name2id(rowid,username) VALUES(1,'owned@chatroom'),(2,'other@chatroom');").unwrap();
        for n in 1..=201i64 {
            c.execute(
                "INSERT INTO name2id(rowid,username) VALUES(?1,?2)",
                params![n + 100, format!("owned-member-{n}")],
            )
            .unwrap();
            c.execute(
                "INSERT INTO contact VALUES(?1,?2,'昵称','备注')",
                params![n + 100, format!("owned-member-{n}")],
            )
            .unwrap();
            c.execute(
                "INSERT INTO chatroom_member(rowid,room_id,member_id) VALUES(?1,1,?2)",
                params![9007199254740992i64 + n, n + 100],
            )
            .unwrap();
        }
        c
    }
    #[test]
    fn exact_group_pages_large_ids_and_changed_boundary() {
        let c = fixture();
        let mut s = LiveGroupSelection {
            username: "owned@chatroom".into(),
            after: None,
        };
        let first = query(&c, &s).unwrap();
        assert_eq!(first.rows.len(), 200);
        assert!(first.has_more);
        assert_eq!(first.rows[0].anchor.row_id, "9007199254740993");
        assert_eq!(first.rows[0].nickname.as_deref(), Some("昵称"));
        s.after = Some(first.rows.last().unwrap().anchor.clone());
        let last = query(&c, &s).unwrap();
        assert_eq!(last.rows.len(), 1);
        assert!(!last.has_more);
        s.username = "other@chatroom".into();
        assert!(matches!(query(&c, &s), Err("LIVE_GROUP_ANCHOR_CHANGED")));
        s.username = "owned@chatroom".into();
        c.execute(
            "UPDATE chatroom_member SET member_id=101 WHERE rowid=?1",
            [s.after.as_ref().unwrap().key().unwrap()],
        )
        .unwrap();
        assert!(matches!(query(&c, &s), Err("LIVE_GROUP_ANCHOR_CHANGED")));
    }
    #[test]
    fn missing_and_invalid_member_metadata_remains_unknown_without_dropping_rows() {
        let c = fixture();
        c.execute_batch("DELETE FROM name2id WHERE rowid=101; UPDATE name2id SET username=CAST(X'FF' AS TEXT) WHERE rowid=102; UPDATE name2id SET username='bad'||char(0)||'id' WHERE rowid=103; UPDATE contact SET nick_name=replace(hex(zeroblob(1025)),'00','x') WHERE id=104; UPDATE contact SET username='different-person' WHERE id=105;").unwrap();
        let result = query(
            &c,
            &LiveGroupSelection {
                username: "owned@chatroom".into(),
                after: None,
            },
        )
        .unwrap();
        assert_eq!(result.rows.len(), 200);
        for r in &result.rows[..3] {
            assert!(r.anchor.username.is_none());
            assert!(r.nickname.is_none());
            assert!(r.remark.is_none());
        }
        assert!(result.rows[3].nickname.is_none());
        assert!(result.rows[3].remark.is_some());
        assert!(result.rows[4].nickname.is_none());
        assert!(result.rows[4].anchor.username.is_some());
    }
    #[test]
    fn group_identity_ambiguity_unsupported_schema_and_non_group_are_rejected() {
        let c = fixture();
        let mut s = LiveGroupSelection {
            username: "filehelper".into(),
            after: None,
        };
        assert!(matches!(query(&c, &s), Err("LIVE_GROUP_SELECTION_INVALID")));
        s.username = "absent@chatroom".into();
        assert!(matches!(query(&c, &s), Err("LIVE_GROUP_NOT_IN_CACHE")));
        s.username = "owned@chatroom".into();
        c.execute_batch("UPDATE chat_room SET id=3 WHERE id=1")
            .unwrap();
        assert!(matches!(
            query(&c, &s),
            Err("LIVE_GROUP_IDENTITY_UNVERIFIED")
        ));
        c.execute_batch("UPDATE chat_room SET id=1 WHERE id=3; INSERT INTO chat_room VALUES(1,'owned@chatroom',NULL)").unwrap();
        assert!(matches!(
            query(&c, &s),
            Err("LIVE_GROUP_IDENTITY_UNVERIFIED")
        ));
        c.execute_batch("DROP TABLE chatroom_member; CREATE TABLE chatroom_member(rowid INTEGER,room_id INTEGER,member_id INTEGER)").unwrap();
        assert!(matches!(
            query(&c, &s),
            Err("LIVE_GROUP_SCHEMA_UNSUPPORTED")
        ));
    }
}
