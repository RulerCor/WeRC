mod codec;
mod schema;
mod snapshot;
pub mod stream;
pub use stream::read_database as stream_database;

use rusqlite::{Connection, ffi, serialize::OwnedData};
use sha2::{Digest, Sha256};
use std::{path::Path, ptr::NonNull};
use werc_domain::history::{HistoryBundle, HistoryReadRequest};
use zeroize::Zeroizing;

pub fn read_database(request: &HistoryReadRequest) -> Result<HistoryBundle, &'static str> {
    let (bytes, wal) = snapshot::snapshot(Path::new(&request.path))?;
    let mut digest = Sha256::new();
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes.as_slice());
    digest.update(wal.as_slice());
    let source_sha256 = hex::encode(digest.finalize());
    let plain = bytes.starts_with(b"SQLite format 3\0");
    if plain && (!request.allow_plaintext || request.key_hex.is_some()) {
        return Err("PLAINTEXT_NOT_AUTHORIZED");
    }
    if bytes.len() < codec::PAGE_SIZE {
        return Err("DATABASE_FORMAT_UNSUPPORTED");
    }
    let decoder = if plain {
        None
    } else {
        Some(codec::PageDecoder::new(
            request.key_hex.as_deref().ok_or("KEY_REQUIRED")?,
            &bytes,
        )?)
    };
    let mut data = codec::decode_database(&bytes, decoder.as_ref())?;
    if plain && (data[16..18] != [0x10, 0] || !matches!(data[20], 0 | 80)) {
        return Err("DATABASE_FORMAT_UNSUPPORTED");
    }
    let wal_committed_frames = codec::apply_wal(&mut data, &wal, decoder.as_ref())?;
    if data.len() < 100 {
        return Err("DATABASE_FORMAT_UNSUPPORTED");
    }
    // 在独立内存视图中转换WAL标记；不修改来源或把明文写到磁盘。
    data[18] = 1;
    data[19] = 1;
    let mut connection = Connection::open_in_memory().map_err(|_| "DATABASE_OPEN_FAILED")?;
    connection
        .set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            2 * 1024 * 1024,
        )
        .map_err(|_| "LIMIT_CONFIGURATION_FAILED")?;
    connection
        .set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_COLUMN, 256)
        .map_err(|_| "LIMIT_CONFIGURATION_FAILED")?;
    connection
        .set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 16384)
        .map_err(|_| "LIMIT_CONFIGURATION_FAILED")?;
    let ptr = unsafe { ffi::sqlite3_malloc64(data.len() as u64) } as *mut u8;
    let ptr = NonNull::new(ptr).ok_or("MEMORY_LIMIT_EXCEEDED")?;
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.as_ptr(), data.len());
    }
    let owned = unsafe { OwnedData::from_raw_nonnull(ptr, data.len()) };
    connection
        .deserialize("main", owned, true)
        .map_err(|_| "DATABASE_OPEN_FAILED")?;
    connection
        .execute_batch("PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; PRAGMA temp_store=MEMORY;")
        .map_err(|_| "READ_ONLY_REQUIRED")?;
    schema::ensure_supported(&connection)?;
    codec::check_database(&connection)?;
    let bundle = schema::read(
        &connection,
        source_sha256,
        if plain {
            "plaintext_explicit"
        } else {
            "sqlcipher4_rawkey_4096_sha512"
        }
        .into(),
        wal_committed_frames,
    )?;
    let encoded =
        Zeroizing::new(serde_json::to_vec(&bundle).map_err(|_| "OUTPUT_ENCODING_FAILED")?);
    if encoded.len() > 32 * 1024 * 1024 {
        return Err("RESULT_LIMIT_EXCEEDED");
    }
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_schema_is_not_silent_empty_success() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE unrelated(id INTEGER)")
            .unwrap();
        assert_eq!(
            schema::read(&c, "0".repeat(64), "test".into(), 0).unwrap_err(),
            "SCHEMA_UNSUPPORTED"
        );
    }
    #[test]
    fn streams_empty_supported_schema_without_empty_batches() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE contact(username TEXT)")
            .unwrap();
        let mut kinds = Vec::new();
        schema::read_stream(
            &c,
            schema::StreamMetadata {
                source_sha256: "0".repeat(64),
                encryption: "test".into(),
                wal_committed_frames: 0,
            },
            &mut |event| {
                kinds.push(match event {
                    werc_domain::history::HistoryReadEvent::Begin { .. } => "begin",
                    werc_domain::history::HistoryReadEvent::End {
                        batches: 0,
                        message_count: 0,
                        contact_count: 0,
                        session_count: 0,
                    } => "end",
                    _ => panic!("unexpected event"),
                });
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(kinds, ["begin", "end"]);
    }
    #[test]
    fn refuses_oversize_encoded_item_without_truncation_or_success_end() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE contact(username TEXT)")
            .unwrap();
        c.execute("INSERT INTO contact VALUES(?)", ["\"".repeat(1024 * 1024)])
            .unwrap();
        let mut ended = false;
        let error = schema::read_stream(
            &c,
            schema::StreamMetadata {
                source_sha256: "0".repeat(64),
                encryption: "test".into(),
                wal_committed_frames: 0,
            },
            &mut |event| {
                ended |= matches!(event, werc_domain::history::HistoryReadEvent::End { .. });
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error, "RESULT_LIMIT_EXCEEDED");
        assert!(!ended);
    }
    #[test]
    fn byte_budget_flushes_before_row_budget() {
        use werc_domain::history::{HISTORY_BATCH_BYTES, HistoryReadEvent};
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE contact(username TEXT)")
            .unwrap();
        let large = "x".repeat(220000);
        for _ in 0..20 {
            c.execute("INSERT INTO contact VALUES(?)", [&large])
                .unwrap();
        }
        let mut counts = Vec::new();
        schema::read_stream(
            &c,
            schema::StreamMetadata {
                source_sha256: "0".repeat(64),
                encryption: "test".into(),
                wal_committed_frames: 0,
            },
            &mut |event| {
                if let HistoryReadEvent::Batch {
                    sequence,
                    digest,
                    data,
                } = event
                {
                    assert_eq!(sequence as usize, counts.len());
                    let encoded = serde_json::to_vec(&data).unwrap();
                    assert!(encoded.len() <= HISTORY_BATCH_BYTES);
                    assert_eq!(digest, hex::encode(Sha256::digest(&encoded)));
                    counts.push(data.contacts.len());
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(counts.len() >= 3);
        assert_eq!(counts.iter().sum::<usize>(), 20);
    }
    #[test]
    fn identity_map_has_explicit_string_budget() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE Name2Id(user_name TEXT); CREATE TABLE contact(username TEXT)",
        )
        .unwrap();
        c.execute("INSERT INTO Name2Id VALUES(?)", ["x".repeat(4097)])
            .unwrap();
        let error = schema::read_stream(
            &c,
            schema::StreamMetadata {
                source_sha256: "0".repeat(64),
                encryption: "test".into(),
                wal_committed_frames: 0,
            },
            &mut |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error, "DATABASE_LIMIT_EXCEEDED");
    }
    #[test]
    fn reads_message_contact_and_session_columns() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE contact(username TEXT,nick_name TEXT); INSERT INTO contact VALUES('synthetic','合成联系人'); CREATE TABLE SessionTable(username TEXT,last_timestamp INTEGER); INSERT INTO SessionTable VALUES('synthetic',1700000000); CREATE TABLE Msg_0123456789abcdef0123456789abcdef(local_id INTEGER,create_time INTEGER,local_type INTEGER,message_content TEXT,server_id INTEGER); INSERT INTO Msg_0123456789abcdef0123456789abcdef VALUES(1,1700000000,1,'<not-html>',9223372036854775806);").unwrap();
        let bundle = schema::read(&c, "0".repeat(64), "test".into(), 0).unwrap();
        assert_eq!(bundle.messages[0].server_id, "9223372036854775806");
        assert_eq!(bundle.messages[0].text.as_deref(), Some("<not-html>"));
        assert!(bundle.messages[0].conversation_id.starts_with("table:"));
        assert_eq!(bundle.contacts.len(), 1);
        assert_eq!(bundle.sessions.len(), 1);
    }
}
