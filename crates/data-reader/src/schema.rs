use crate::codec::{ReadResult, sqlite_error};
use md5::{Digest, Md5};
use rusqlite::{Connection, types::ValueRef};
use sha2::Sha256;
use std::{collections::BTreeMap, io::Read};
use werc_domain::history::{
    HISTORY_BATCH_BYTES, HISTORY_BATCH_ROWS, HistoryBundle, HistoryContact, HistoryMessage,
    HistoryReadEvent, HistorySession,
};
use zeroize::Zeroizing;

// Schema availability is separate from integrity of data we actually support.
// Do not run a generic SQLite checker over unknown WCDB index layouts and then
// claim they are corrupt, or publish any rows before the supported DB is checked.
pub(crate) fn ensure_supported(c: &Connection) -> ReadResult<()> {
    let found: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND (name='contact' OR name='SessionTable' OR substr(name,1,4)='Msg_'))",
        [], |r| r.get(0),
    ).map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?;
    if found {
        Ok(())
    } else {
        Err("SCHEMA_UNSUPPORTED")
    }
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn columns(c: &Connection, table: &str) -> ReadResult<Vec<String>> {
    if !valid_name(table) {
        return Err("SCHEMA_UNSUPPORTED");
    }
    c.prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))
}
fn require(cols: &[String], names: &[&str]) -> ReadResult<()> {
    if names.iter().all(|n| cols.iter().any(|c| c == n)) {
        Ok(())
    } else {
        Err("SCHEMA_UNSUPPORTED")
    }
}
fn optional(cols: &[String], name: &str) -> String {
    if cols.iter().any(|n| n == name) {
        format!("\"{name}\"")
    } else {
        "NULL".into()
    }
}
// Independent adapter, not an upstream implementation. Whitelist evidence:
// WeFlow 93d46a3 chatService getRowField (2982), message timestamp (11557),
// and message-cursor content (10735). Contact/session-only localType aliases
// are intentionally NOT recognized as Msg_ columns.
// These are returned-field aliases, NOT proof of a particular on-disk schema.
// Apply only inside an already recognized Msg_<32 hex> table; no schema guessing.
fn message_column(cols: &[String], candidates: &[&str], required: bool) -> ReadResult<String> {
    let selected = candidates
        .iter()
        .find_map(|candidate| cols.iter().find(|name| name.as_str() == *candidate))
        .or_else(|| {
            candidates.iter().find_map(|candidate| {
                cols.iter()
                    .find(|name| name.eq_ignore_ascii_case(candidate))
            })
        });
    match selected {
        Some(name) if valid_name(name) => Ok(format!("\"{name}\"")),
        Some(_) => Err("SCHEMA_UNSUPPORTED"),
        None if required => Err("SCHEMA_UNSUPPORTED"),
        None => Ok("NULL".into()),
    }
}

// Check the tiny zstd frame header BEFORE constructing a decoder: output.take()
// alone does not bound the decoder's advertised history-window allocation.
fn zstd_budget(bytes: &[u8]) -> ReadResult<()> {
    let descriptor = *bytes.get(4).ok_or("COMPRESSED_CONTENT_INVALID")?;
    let single = descriptor & 0x20 != 0;
    let size_flag = descriptor >> 6;
    let mut offset = 5usize;
    if !single {
        let window = *bytes.get(offset).ok_or("COMPRESSED_CONTENT_INVALID")?;
        offset += 1;
        let base = 1u64 << (10 + (window >> 3));
        let size = base + (base >> 3) * u64::from(window & 7);
        if size > 8 * 1024 * 1024 {
            return Err("DATABASE_LIMIT_EXCEEDED");
        }
    }
    offset += [0, 1, 2, 4][(descriptor & 3) as usize];
    let size_bytes = match size_flag {
        0 if single => 1,
        0 => 0,
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let encoded = bytes
        .get(offset..offset + size_bytes)
        .ok_or("COMPRESSED_CONTENT_INVALID")?;
    if size_bytes != 0 {
        let mut size = 0u64;
        for (index, byte) in encoded.iter().enumerate() {
            size |= u64::from(*byte) << (index * 8);
        }
        if size_flag == 1 {
            size += 256;
        }
        if size > 1024 * 1024 {
            return Err("CONTENT_LIMIT_EXCEEDED");
        }
    }
    Ok(())
}

fn text(row: &rusqlite::Row<'_>, i: usize, remaining: &mut usize) -> ReadResult<Option<String>> {
    let value = row
        .get_ref(i)
        .map_err(|e| sqlite_error(e, "DATABASE_READ_FAILED"))?;
    let bytes = match value {
        ValueRef::Null => return Ok(None),
        ValueRef::Text(b) | ValueRef::Blob(b) => b,
        _ => return Err("SCHEMA_UNSUPPORTED"),
    };
    if bytes.len() > 1024 * 1024 {
        return Err("CONTENT_LIMIT_EXCEEDED");
    }
    let decoded = if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        zstd_budget(bytes)?;
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(bytes)
            .map_err(|_| "COMPRESSED_CONTENT_INVALID")?;
        let mut output = Vec::new();
        decoder
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut output)
            .map_err(|_| "COMPRESSED_CONTENT_INVALID")?;
        if output.len() > 1024 * 1024 {
            return Err("CONTENT_LIMIT_EXCEEDED");
        }
        output
    } else {
        bytes.to_vec()
    };
    *remaining = remaining
        .checked_sub(decoded.len())
        .ok_or("CONTENT_LIMIT_EXCEEDED")?;
    Ok(String::from_utf8(decoded).ok())
}
fn integer(row: &rusqlite::Row<'_>, i: usize) -> ReadResult<i64> {
    row.get(i)
        .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))
}
fn server_id(row: &rusqlite::Row<'_>, i: usize) -> ReadResult<String> {
    match row
        .get_ref(i)
        .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?
    {
        ValueRef::Null => Ok("0".into()),
        ValueRef::Integer(v) => Ok(v.to_string()),
        ValueRef::Text(v) if v.len() <= 32 && v.iter().all(u8::is_ascii_digit) => {
            String::from_utf8(v.to_vec()).map_err(|_| "SCHEMA_UNSUPPORTED")
        }
        _ => Err("SCHEMA_UNSUPPORTED"),
    }
}
fn rows(
    c: &Connection,
    sql: &str,
    streaming: bool,
    mut visitor: impl FnMut(&rusqlite::Row<'_>) -> ReadResult<()>,
) -> ReadResult<()> {
    // Streaming needs no global ordering. Avoid an unbounded SQLite sorting
    // workspace (and never spill plaintext into a temporary file).
    let sql = if streaming {
        sql.split(" ORDER BY ")
            .next()
            .unwrap_or(sql)
            .replace(" LIMIT 50001", "")
    } else {
        sql.to_owned()
    };
    let mut statement = c
        .prepare(&sql)
        .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?;
    let mut query = statement
        .query([])
        .map_err(|e| sqlite_error(e, "DATABASE_READ_FAILED"))?;
    let mut count = 0;
    while let Some(row) = query
        .next()
        .map_err(|e| sqlite_error(e, "DATABASE_READ_FAILED"))?
    {
        count += 1;
        if !streaming && count > 50000 {
            return Err("ROW_LIMIT_EXCEEDED");
        }
        visitor(row)?;
    }
    Ok(())
}

pub fn read(
    c: &Connection,
    source_sha256: String,
    encryption: String,
    wal_committed_frames: usize,
) -> ReadResult<HistoryBundle> {
    let mut collected = HistoryBundle {
        schema: "wechat4_columns_v1".into(),
        source_sha256: source_sha256.clone(),
        encryption: encryption.clone(),
        wal_committed_frames,
        messages: vec![],
        contacts: vec![],
        sessions: vec![],
        warnings: vec![],
    };
    // Legacy entry remains bounded; only v3 streams arbitrarily many bounded batches.
    traverse(
        c,
        source_sha256,
        encryption,
        wal_committed_frames,
        false,
        &mut |event| {
            if let HistoryReadEvent::Batch { mut data, .. } = event {
                collected.messages.append(&mut data.messages);
                collected.contacts.append(&mut data.contacts);
                collected.sessions.append(&mut data.sessions);
                collected.warnings = data.warnings;
            }
            Ok(())
        },
    )?;
    Ok(collected)
}

pub struct StreamMetadata {
    pub source_sha256: String,
    pub encryption: String,
    pub wal_committed_frames: usize,
}
pub fn read_stream(
    c: &Connection,
    meta: StreamMetadata,
    visitor: &mut dyn FnMut(HistoryReadEvent) -> ReadResult<()>,
) -> ReadResult<()> {
    traverse(
        c,
        meta.source_sha256,
        meta.encryption,
        meta.wal_committed_frames,
        true,
        visitor,
    )
}

// Stop serialization at the wire budget rather than allocating a potentially
// much larger escaped string (four 1 MiB fields can otherwise expand to 24 MiB).
fn encode_bounded<T: serde::Serialize>(value: &T) -> ReadResult<Zeroizing<Vec<u8>>> {
    struct Limited {
        bytes: Zeroizing<Vec<u8>>,
        exceeded: bool,
    }
    impl std::io::Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > HISTORY_BATCH_BYTES.saturating_sub(self.bytes.len()) {
                self.exceeded = true;
                return Err(std::io::Error::other("batch budget"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Limited {
        bytes: Zeroizing::new(Vec::new()),
        exceeded: false,
    };
    if serde_json::to_writer(&mut writer, value).is_err() {
        return Err(if writer.exceeded {
            "RESULT_LIMIT_EXCEEDED"
        } else {
            "OUTPUT_ENCODING_FAILED"
        });
    }
    Ok(writer.bytes)
}

struct Batches<'a> {
    data: HistoryBundle,
    visitor: &'a mut dyn FnMut(HistoryReadEvent) -> ReadResult<()>,
    sequence: u64,
    counts: [usize; 3],
    bytes: usize,
    rows: usize,
}
impl Batches<'_> {
    fn flush(&mut self) -> ReadResult<()> {
        if self.rows == 0 {
            return Ok(());
        }
        let encoded = encode_bounded(&self.data)?;
        if encoded.len() > HISTORY_BATCH_BYTES {
            return Err("RESULT_LIMIT_EXCEEDED");
        }
        let digest = hex::encode(Sha256::digest(encoded.as_slice()));
        drop(encoded);
        let next = HistoryBundle {
            schema: self.data.schema.clone(),
            source_sha256: self.data.source_sha256.clone(),
            encryption: self.data.encryption.clone(),
            wal_committed_frames: self.data.wal_committed_frames,
            messages: vec![],
            contacts: vec![],
            sessions: vec![],
            warnings: self.data.warnings.clone(),
        };
        // Do not clone a filled batch; only the bounded metadata is duplicated.
        let data = std::mem::replace(&mut self.data, next);
        (self.visitor)(HistoryReadEvent::Batch {
            sequence: self.sequence,
            digest,
            data,
        })?;
        self.sequence += 1;
        self.bytes = encode_bounded(&self.data)?.len();
        self.rows = 0;
        Ok(())
    }
    fn reserve<T: serde::Serialize>(&mut self, item: &T, kind: usize) -> ReadResult<()> {
        let encoded = encode_bounded(item)?;
        // The baseline already includes every bounded warning and metadata field.
        // One byte per row conservatively accounts for each JSON array comma.
        let size = encoded.len() + 1;
        if self.rows == HISTORY_BATCH_ROWS || self.bytes + size > HISTORY_BATCH_BYTES {
            self.flush()?;
        }
        if self.bytes + size > HISTORY_BATCH_BYTES {
            return Err("RESULT_LIMIT_EXCEEDED");
        }
        self.bytes += size;
        self.rows += 1;
        self.counts[kind] = self.counts[kind]
            .checked_add(1)
            .ok_or("RESULT_LIMIT_EXCEEDED")?;
        Ok(())
    }
    fn message(&mut self, value: HistoryMessage) -> ReadResult<()> {
        self.reserve(&value, 0)?;
        self.data.messages.push(value);
        Ok(())
    }
    fn contact(&mut self, value: HistoryContact) -> ReadResult<()> {
        self.reserve(&value, 1)?;
        self.data.contacts.push(value);
        Ok(())
    }
    fn session(&mut self, value: HistorySession) -> ReadResult<()> {
        self.reserve(&value, 2)?;
        self.data.sessions.push(value);
        Ok(())
    }
}

fn traverse(
    c: &Connection,
    source_sha256: String,
    encryption: String,
    wal_committed_frames: usize,
    streaming: bool,
    visitor: &mut dyn FnMut(HistoryReadEvent) -> ReadResult<()>,
) -> ReadResult<()> {
    let names = {
        let mut statement = c.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' LIMIT 4097").map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?;
        let mut cursor = statement
            .query([])
            .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?;
        let mut names = Vec::new();
        while let Some(row) = cursor
            .next()
            .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?
        {
            if names.len() == 4096 {
                return Err("SCHEMA_LIMIT_EXCEEDED");
            }
            let name: &str = row
                .get_ref(0)
                .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?
                .as_str()
                .map_err(|_| "SCHEMA_UNSUPPORTED")?;
            if name.len() > 128 {
                return Err("SCHEMA_LIMIT_EXCEEDED");
            }
            names.push(name.to_owned());
        }
        // Sorting at most 4096 small names on the Rust side is explicitly bounded.
        names.sort();
        names
    };
    let bundle = HistoryBundle {
        schema: "wechat4_columns_v1".into(),
        source_sha256,
        encryption,
        wal_committed_frames,
        messages: Vec::new(),
        contacts: Vec::new(),
        sessions: Vec::new(),
        warnings: Vec::new(),
    };
    visitor(HistoryReadEvent::Begin {
        schema: bundle.schema.clone(),
        source_sha256: bundle.source_sha256.clone(),
        encryption: bundle.encryption.clone(),
        wal_committed_frames,
    })?;
    let mut output = Batches {
        data: bundle,
        visitor,
        sequence: 0,
        counts: [0; 3],
        bytes: 512,
        rows: 0,
    };
    let mut remaining = 32 * 1024 * 1024;
    let mut mapping_bytes = 0usize;
    let mut mapping_entries = 0usize;
    let mut identities = BTreeMap::new();
    let mut conversations = BTreeMap::new();
    for name in &names {
        if name == "Name2Id" || name.starts_with("Name2Id_") {
            let cols = columns(c, name)?;
            require(&cols, &["user_name"])?;
            rows(
                c,
                &format!("SELECT rowid,user_name FROM \"{name}\" LIMIT 50001"),
                streaming,
                |r| {
                    if let Some(username) = text(r, 1, &mut remaining)? {
                        mapping_entries += 1;
                        mapping_bytes = mapping_bytes
                            .checked_add(username.len() * 2 + 256)
                            .ok_or("DATABASE_LIMIT_EXCEEDED")?;
                        if username.len() > 4096
                            || mapping_entries > 100000
                            || mapping_bytes > 32 * 1024 * 1024
                        {
                            return Err("DATABASE_LIMIT_EXCEEDED");
                        }
                        conversations.insert(
                            format!("Msg_{:x}", Md5::digest(username.as_bytes())),
                            username.clone(),
                        );
                        identities.insert(integer(r, 0)?, username);
                    }
                    Ok(())
                },
            )?;
        }
    }
    for table in names.iter().filter(|table| table.starts_with("Msg_")) {
        if !conversations.contains_key(table) && output.data.warnings.len() < 128 {
            output
                .data
                .warnings
                .push(format!("IDENTITY_UNRESOLVED:{table}"));
        }
    }
    output.bytes = encode_bounded(&output.data)?.len();
    for table in &names {
        if table == "contact" {
            let cols = columns(c, table)?;
            require(&cols, &["username"])?;
            let sql = format!(
                "SELECT username,{},{},{} FROM contact ORDER BY username LIMIT 50001",
                optional(&cols, "nick_name"),
                optional(&cols, "remark"),
                optional(&cols, "alias")
            );
            rows(c, &sql, streaming, |r| {
                if streaming {
                    remaining = 8 * 1024 * 1024;
                }
                let username = text(r, 0, &mut remaining)?.ok_or("SCHEMA_UNSUPPORTED")?;
                output.contact(HistoryContact {
                    username,
                    nickname: text(r, 1, &mut remaining)?,
                    remark: text(r, 2, &mut remaining)?,
                    alias: text(r, 3, &mut remaining)?,
                })?;
                Ok(())
            })?;
        } else if table == "SessionTable" {
            let cols = columns(c, table)?;
            require(&cols, &["username", "last_timestamp"])?;
            let sql = format!(
                "SELECT username,last_timestamp,{},{} FROM SessionTable ORDER BY last_timestamp,username LIMIT 50001",
                optional(&cols, "summary"),
                optional(&cols, "unread_count")
            );
            rows(c, &sql, streaming, |r| {
                if streaming {
                    remaining = 8 * 1024 * 1024;
                }
                output.session(HistorySession {
                    username: text(r, 0, &mut remaining)?.ok_or("SCHEMA_UNSUPPORTED")?,
                    timestamp: integer(r, 1)?,
                    summary: text(r, 2, &mut remaining)?,
                    unread_count: r
                        .get(3)
                        .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?,
                })?;
                Ok(())
            })?;
        } else if let Some(suffix) = table.strip_prefix("Msg_") {
            if suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("SCHEMA_UNSUPPORTED");
            }
            let cols = columns(c, table)?;
            let local_id = message_column(&cols, &["local_id"], true)?;
            let created = message_column(&cols, &["create_time", "createTime"], true)?;
            let kind = message_column(&cols, &["local_type"], true)?;
            let content = message_column(
                &cols,
                &["message_content", "messageContent", "content"],
                true,
            )?;
            let sql = format!(
                "SELECT {local_id},{created},{kind},{content},{},{},{},{} FROM \"{table}\" ORDER BY {created},{local_id} LIMIT 50001",
                message_column(&cols, &["server_id"], false)?,
                message_column(&cols, &["sort_seq"], false)?,
                message_column(&cols, &["real_sender_id"], false)?,
                message_column(&cols, &["compress_content"], false)?,
            );
            let conversation_id = conversations
                .get(table)
                .cloned()
                .unwrap_or_else(|| format!("table:{table}"));
            rows(c, &sql, streaming, |r| {
                if streaming {
                    remaining = 8 * 1024 * 1024;
                }
                if !streaming && output.counts[0] >= 50000 {
                    return Err("ROW_LIMIT_EXCEEDED");
                }
                let local_id = integer(r, 0)?;
                let timestamp = integer(r, 1)?;
                let compressed = text(r, 7, &mut remaining)?;
                let text = match compressed.filter(|s| !s.is_empty()) {
                    Some(content) => Some(content),
                    None => text(r, 3, &mut remaining)?,
                };
                let sender: Option<i64> = r
                    .get(6)
                    .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?;
                let sequence: Option<i64> = r
                    .get(5)
                    .map_err(|e| sqlite_error(e, "SCHEMA_UNSUPPORTED"))?;
                output.message(HistoryMessage {
                    id: format!("{table}:{local_id}"),
                    conversation_id: conversation_id.clone(),
                    sender_id: sender.and_then(|v| identities.get(&v).cloned()),
                    local_id,
                    server_id: server_id(r, 4)?,
                    timestamp,
                    sequence: sequence.unwrap_or(local_id),
                    message_type: integer(r, 2)?,
                    content_status: if text.is_some() {
                        "decoded"
                    } else {
                        "binary_or_missing"
                    }
                    .into(),
                    text,
                })?;
                Ok(())
            })?;
        }
    }
    if !names
        .iter()
        .any(|n| n == "contact" || n == "SessionTable" || n.starts_with("Msg_"))
    {
        return Err("SCHEMA_UNSUPPORTED");
    }
    output.flush()?;
    (output.visitor)(HistoryReadEvent::End {
        batches: output.sequence,
        message_count: output.counts[0],
        contact_count: output.counts[1],
        session_count: output.counts[2],
    })
}

#[cfg(test)]
mod compression_budget_tests {
    use super::*;
    #[test]
    fn rejects_huge_zstd_windows_before_decoder_allocation() {
        assert_eq!(
            zstd_budget(&[0x28, 0xb5, 0x2f, 0xfd, 0, 0xf8]),
            Err("DATABASE_LIMIT_EXCEEDED")
        );
        assert_eq!(
            zstd_budget(&[
                0x28, 0xb5, 0x2f, 0xfd, 0xe0, 255, 255, 255, 255, 255, 255, 255, 255
            ]),
            Err("CONTENT_LIMIT_EXCEEDED")
        );
        assert_eq!(
            zstd_budget(&[0x28, 0xb5, 0x2f, 0xfd, 0xe0]),
            Err("COMPRESSED_CONTENT_INVALID")
        );
        let frame = [
            0x28, 0xb5, 0x2f, 0xfd, 0x20, 3, 0x19, 0, 0, b'a', b'b', b'c',
        ];
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE contact(username BLOB)")
            .unwrap();
        c.execute("INSERT INTO contact VALUES(?)", [frame.as_slice()])
            .unwrap();
        assert_eq!(
            read(&c, "0".repeat(64), "test".into(), 0).unwrap().contacts[0].username,
            "abc"
        );
        assert!(zstd_budget(&[0x28, 0xb5, 0x2f, 0xfd, 0x20, 3]).is_ok());
    }
}

#[cfg(test)]
mod message_compatibility_tests {
    use super::*;
    const TABLE: &str = "Msg_0123456789abcdef0123456789abcdef";
    fn streamed(c: &Connection) -> ReadResult<Vec<HistoryMessage>> {
        let mut messages = Vec::new();
        let mut ended = false;
        read_stream(
            c,
            StreamMetadata {
                source_sha256: "0".repeat(64),
                encryption: "test".into(),
                wal_committed_frames: 0,
            },
            &mut |event| {
                match event {
                    HistoryReadEvent::Batch { mut data, .. } => messages.append(&mut data.messages),
                    HistoryReadEvent::End { message_count, .. } => {
                        assert_eq!(message_count, messages.len());
                        ended = true;
                    }
                    _ => (),
                }
                Ok(())
            },
        )?;
        assert!(ended);
        Ok(messages)
    }
    #[test]
    fn explicit_message_timestamp_and_content_aliases_are_read() {
        // Synthetic adapter support, not evidence of a real client's disk layout.
        for created in ["create_time", "createTime"] {
            for content in ["message_content", "messageContent", "content"] {
                let c = Connection::open_in_memory().unwrap();
                c.execute_batch(&format!("CREATE TABLE {TABLE}(local_id INTEGER,{created} INTEGER,local_type INTEGER,{content} TEXT,server_id INTEGER); INSERT INTO {TABLE} VALUES(7,1700000000,49,'<msg><title>raw XML</title></msg>',9223372036854775806)")).unwrap();
                let messages = streamed(&c).unwrap();
                assert_eq!(messages.len(), 1);
                let message = &messages[0];
                assert_eq!(message.local_id, 7);
                assert_eq!(message.timestamp, 1700000000);
                assert_eq!(message.message_type, 49);
                assert_eq!(message.server_id, "9223372036854775806");
                assert_eq!(
                    message.text.as_deref(),
                    Some("<msg><title>raw XML</title></msg>")
                );
                assert_eq!(
                    read(&c, "0".repeat(64), "test".into(), 0).unwrap().messages[0].text,
                    message.text
                );
            }
        }
    }
    #[test]
    fn missing_required_message_columns_remain_unsupported() {
        let fields = [
            "local_id INTEGER",
            "createTime INTEGER",
            "local_type INTEGER",
            "messageContent TEXT",
        ];
        for missing in 0..fields.len() {
            let c = Connection::open_in_memory().unwrap();
            let kept = fields
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != missing)
                .map(|(_, field)| *field)
                .collect::<Vec<_>>()
                .join(",");
            c.execute_batch(&format!("CREATE TABLE {TABLE}({kept})"))
                .unwrap();
            assert_eq!(streamed(&c).unwrap_err(), "SCHEMA_UNSUPPORTED");
        }
        // localType is evidenced only on contact/session return rows, so do NOT
        // extrapolate it (or WCDB_CT_local_type) to a Msg_ table.
        for unsupported in ["localType", "WCDB_CT_local_type"] {
            let c = Connection::open_in_memory().unwrap();
            c.execute_batch(&format!("CREATE TABLE {TABLE}(local_id INTEGER,create_time INTEGER,{unsupported} INTEGER,message_content TEXT)")).unwrap();
            assert_eq!(streamed(&c).unwrap_err(), "SCHEMA_UNSUPPORTED");
        }
    }
    #[test]
    fn canonical_message_columns_win_when_aliases_coexist() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&format!("CREATE TABLE {TABLE}(local_id INTEGER,create_time INTEGER,createTime INTEGER,local_type INTEGER,message_content TEXT,messageContent TEXT,content TEXT); INSERT INTO {TABLE} VALUES(1,1700000000,2,1,'canonical','camel alias','generic alias')")).unwrap();
        let message = streamed(&c).unwrap().remove(0);
        assert_eq!(message.timestamp, 1700000000);
        assert_eq!(message.text.as_deref(), Some("canonical"));
    }
    #[test]
    fn valid_compressed_content_does_not_parse_invalid_original_field() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&format!("CREATE TABLE {TABLE}(local_id INTEGER,create_time INTEGER,local_type INTEGER,message_content,compress_content BLOB)")).unwrap();
        // Independent raw zstd frame containing abc. The primary field is an
        // INTEGER, which text() would reject if evaluated eagerly.
        let frame = [
            0x28, 0xb5, 0x2f, 0xfd, 0x20, 3, 0x19, 0, 0, b'a', b'b', b'c',
        ];
        c.execute(
            &format!("INSERT INTO {TABLE} VALUES(1,1700000000,1,99,?)"),
            [frame.as_slice()],
        )
        .unwrap();
        assert_eq!(streamed(&c).unwrap()[0].text.as_deref(), Some("abc"));
        assert_eq!(
            read(&c, "0".repeat(64), "test".into(), 0).unwrap().messages[0]
                .text
                .as_deref(),
            Some("abc")
        );
    }
    #[test]
    fn empty_or_null_compressed_content_falls_back_lazily() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&format!("CREATE TABLE {TABLE}(local_id INTEGER,create_time INTEGER,local_type INTEGER,message_content,compress_content); INSERT INTO {TABLE} VALUES(1,1700000000,1,'empty fallback',''),(2,1700000001,1,'null fallback',NULL)")).unwrap();
        let messages = streamed(&c).unwrap();
        assert_eq!(messages[0].text.as_deref(), Some("empty fallback"));
        assert_eq!(messages[1].text.as_deref(), Some("null fallback"));
        c.execute(
            &format!("UPDATE {TABLE} SET message_content=99 WHERE local_id=1"),
            [],
        )
        .unwrap();
        assert_eq!(streamed(&c).unwrap_err(), "SCHEMA_UNSUPPORTED");
    }
}
