//! Exact server ID + timestamp + selected-conversation binding, using the observed VoiceInfo schema.
use crate::{
    live::ReadView,
    live_protocol::{LiveVoiceAnchor, LiveVoicePayload},
};
use rusqlite::Connection;
use std::path::Path;
use zeroize::Zeroizing;

fn query(c: &Connection, anchor: &LiveVoiceAnchor) -> Result<Option<String>, &'static str> {
    let server = anchor.server_key().ok_or("LIVE_VOICE_ANCHOR_INVALID")?;
    for (table, required) in [
        ("Name2Id", &["user_name"][..]),
        (
            "VoiceInfo",
            &[
                "chat_name_id",
                "create_time",
                "local_id",
                "svr_id",
                "voice_data",
                "data_index",
            ][..],
        ),
    ] {
        let exists: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [table],
                |r| r.get(0),
            )
            .map_err(|_| "LIVE_VOICE_SCHEMA_UNSUPPORTED")?;
        if !exists {
            return Err("LIVE_VOICE_SCHEMA_UNSUPPORTED");
        }
        let columns: Vec<String> = c
            .prepare("SELECT name FROM pragma_table_info(?1) LIMIT 65")
            .map_err(|_| "LIVE_VOICE_SCHEMA_UNSUPPORTED")?
            .query_map([table], |r| r.get(0))
            .map_err(|_| "LIVE_VOICE_SCHEMA_UNSUPPORTED")?
            .collect::<Result<_, _>>()
            .map_err(|_| "LIVE_VOICE_SCHEMA_UNSUPPORTED")?;
        if columns.len() > 64 || !required.iter().all(|n| columns.iter().any(|c| c == n)) {
            return Err("LIVE_VOICE_SCHEMA_UNSUPPORTED");
        }
    }
    let mut statement=c.prepare("SELECT length(voice_data),CASE WHEN typeof(voice_data)='blob' AND length(voice_data) BETWEEN 1 AND 262144 THEN voice_data ELSE NULL END FROM VoiceInfo WHERE chat_name_id IN (SELECT rowid FROM Name2Id WHERE user_name=?3) AND svr_id=?1 AND create_time=?2 LIMIT 2").map_err(|_|"LIVE_VOICE_SCHEMA_UNSUPPORTED")?;
    let mut rows = statement
        .query(rusqlite::params![
            server,
            anchor.timestamp,
            anchor.conversation()
        ])
        .map_err(|_| "LIVE_VOICE_READ_FAILED")?;
    let Some(row) = rows.next().map_err(|_| "LIVE_VOICE_READ_FAILED")? else {
        return Ok(None);
    };
    let length: Option<i64> = row.get(0).map_err(|_| "LIVE_VOICE_INVALID")?;
    let bytes = Zeroizing::new(
        row.get::<_, Option<Vec<u8>>>(1)
            .map_err(|_| "LIVE_VOICE_INVALID")?
            .ok_or("LIVE_VOICE_UNAVAILABLE")?,
    );
    if length != Some(bytes.len() as i64) || !bytes.starts_with(b"\x02#!SILK_V3") {
        return Err("LIVE_VOICE_INVALID");
    }
    if rows.next().map_err(|_| "LIVE_VOICE_READ_FAILED")?.is_some() {
        return Err("LIVE_VOICE_AMBIGUOUS");
    }
    Ok(Some(hex::encode(&*bytes)))
}

pub(crate) fn read(
    path: &Path,
    key: &str,
    anchor: &LiveVoiceAnchor,
) -> Result<LiveVoicePayload, &'static str> {
    let view = ReadView::open(path, key)?;
    let silk_hex = query(&view.connection, anchor)?;
    view.check_deadline()?;
    view.connection
        .execute_batch("ROLLBACK")
        .map_err(|_| "LIVE_TRANSACTION_FAILED")?;
    Ok(LiveVoicePayload {
        schema_version: 1,
        read_only: true,
        silk_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE Name2Id(user_name TEXT); INSERT INTO Name2Id VALUES('filehelper'),('other'); CREATE TABLE VoiceInfo(chat_name_id INTEGER,create_time INTEGER,local_id INTEGER,svr_id INTEGER,voice_data BLOB,data_index TEXT);").unwrap();
        c
    }
    fn insert(c: &Connection, chat: i64, time: i64, server: i64, bytes: &[u8]) {
        c.execute(
            "INSERT INTO VoiceInfo VALUES(?1,?2,7,?3,?4,'')",
            rusqlite::params![chat, time, server, bytes],
        )
        .unwrap();
    }
    #[test]
    fn voice_is_bound_to_conversation_server_id_and_timestamp() {
        let c = fixture();
        let b = b"\x02#!SILK_V3\x01\x00\x01";
        let anchor = LiveVoiceAnchor {
            username: None,
            server_id: "9223372036854775000".into(),
            timestamp: 1700000000,
        };
        insert(&c, 2, anchor.timestamp, anchor.server_key().unwrap(), b);
        insert(&c, 1, anchor.timestamp + 1, anchor.server_key().unwrap(), b);
        insert(&c, 1, anchor.timestamp, 42, b);
        assert_eq!(query(&c, &anchor).unwrap(), None);
        insert(&c, 1, anchor.timestamp, anchor.server_key().unwrap(), b);
        assert_eq!(query(&c, &anchor).unwrap(), Some(hex::encode(b)));
        insert(&c, 1, anchor.timestamp, anchor.server_key().unwrap(), b);
        assert_eq!(query(&c, &anchor).unwrap_err(), "LIVE_VOICE_AMBIGUOUS");
    }
    #[test]
    fn explicit_conversation_never_falls_back_to_filehelper() {
        let c = fixture();
        let mut anchor = LiveVoiceAnchor {
            username: Some("other".into()),
            server_id: "42".into(),
            timestamp: 1700000000,
        };
        let first = b"\x02#!SILK_V3\x01\x00\x01";
        let second = b"\x02#!SILK_V3\x01\x00\x02";
        insert(&c, 1, anchor.timestamp, 42, first);
        assert_eq!(query(&c, &anchor).unwrap(), None);
        insert(&c, 2, anchor.timestamp, 42, second);
        assert_eq!(query(&c, &anchor).unwrap(), Some(hex::encode(second)));
        anchor.username = None;
        assert_eq!(query(&c, &anchor).unwrap(), Some(hex::encode(first)));
        anchor.username = Some("other' OR 1=1 --".into());
        assert_eq!(query(&c, &anchor).unwrap(), None);
        anchor.username = Some(String::new());
        assert_eq!(query(&c, &anchor).unwrap_err(), "LIVE_VOICE_ANCHOR_INVALID");
    }
    #[test]
    fn voice_rejects_missing_oversized_text_and_invalid_keys() {
        let anchor = LiveVoiceAnchor {
            username: None,
            server_id: "42".into(),
            timestamp: 1700000000,
        };
        for bytes in [vec![], vec![0; 262145], b"not silk".to_vec()] {
            let c = fixture();
            insert(&c, 1, anchor.timestamp, 42, &bytes);
            assert!(query(&c, &anchor).is_err());
        }
        let c = fixture();
        c.execute(
            "INSERT INTO VoiceInfo VALUES(1,1700000000,7,42,'text','')",
            [],
        )
        .unwrap();
        assert!(query(&c, &anchor).is_err());
        for id in ["0", "-1", "042", "9223372036854775808"] {
            assert!(
                query(
                    &c,
                    &LiveVoiceAnchor {
                        username: None,
                        server_id: id.into(),
                        timestamp: anchor.timestamp
                    }
                )
                .is_err()
            );
        }
    }
}
