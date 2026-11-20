//! Owned encrypted WAL concurrency fixture. Never opens a WeChat source.
//! The fixed synthetic key is intentionally public; this is not a product worker.
#[path = "../src/live.rs"]
mod live;
#[path = "../../../crates/domain/src/live.rs"]
mod live_protocol;
#[path = "../src/text.rs"]
mod text;
#[path = "../src/window.rs"]
mod window;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{BufRead, Read, Write},
    path::{Component, Path},
    time::Duration,
};

fn emit(value: Value) {
    println!("{value}");
    std::io::stdout().flush().unwrap();
}

fn validate(path: &Path) {
    assert!(path.is_absolute());
    assert!(!path.components().any(|c| matches!(c, Component::ParentDir)));
    assert_eq!(path.file_name().unwrap(), "owned.werc-wal-fixture.sqlite3");
    for ancestor in path.ancestors().skip(1) {
        let meta = std::fs::symlink_metadata(ancestor).unwrap();
        assert!(!meta.file_type().is_symlink());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(meta.file_attributes() & 0x400, 0);
        }
    }
}

fn configure(c: &Connection, wrong_key: bool) -> rusqlite::Result<()> {
    // Explicit SQLCipher4 parameters match the existing raw-key page verifier.
    c.execute_batch(&format!(
        "PRAGMA key=\"x'{}'\"; PRAGMA cipher_compatibility=4; \
         PRAGMA cipher_memory_security=ON; PRAGMA temp_store=MEMORY; \
         PRAGMA trusted_schema=OFF; PRAGMA mmap_size=0;",
        if wrong_key { "6b" } else { "5a" }.repeat(32)
    ))?;
    c.busy_timeout(Duration::from_millis(100))?;
    Ok(())
}

fn stats(c: &Connection) -> rusqlite::Result<Value> {
    let marker: String = c.query_row("SELECT label FROM fixture_marker", [], |r| r.get(0))?;
    assert_eq!(marker, "WeRC owned encrypted WAL fixture");
    let (count, sum, latest): (i64, i64, i64) = c.query_row(
        "SELECT count(*),coalesce(sum(id),0),coalesce(max(id),0) FROM fixture_messages",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(json!({"count":count,"sum":sum,"latest":latest}))
}

fn commands(mut f: impl FnMut(&str) -> bool) {
    let mut input = std::io::stdin().lock();
    loop {
        let mut line = String::new();
        // Driver is owned; still bound the fixture control protocol.
        let mut limited = input.by_ref().take(257);
        if limited.read_line(&mut line).unwrap() == 0 {
            break;
        }
        assert!(line.len() <= 256);
        if !f(line.trim()) {
            break;
        }
    }
}

fn writer(path: &Path) {
    live::prepare_memory().unwrap();
    // Refuse an existing destination before any SQLite read/write open.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    let c = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    configure(&c, false).unwrap();
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; \
        CREATE TABLE fixture_marker(label TEXT NOT NULL); \
        INSERT INTO fixture_marker VALUES('WeRC owned encrypted WAL fixture'); \
        CREATE TABLE fixture_messages(id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, body TEXT NOT NULL);\
        CREATE TABLE Msg_9e20f478899dc29eb19741386f9343c8(local_id INTEGER PRIMARY KEY, create_time INTEGER NOT NULL, local_type INTEGER NOT NULL, server_id INTEGER, message_content, compress_content); \
        CREATE INDEX fixture_msg_order ON Msg_9e20f478899dc29eb19741386f9343c8(create_time,local_id);").unwrap();
    let sqlite: String = c
        .query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap();
    let cipher: String = c
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    emit(json!({"ready":true,"sqlite":sqlite,"cipher":cipher}));
    commands(|line| {
        let (op, amount) = line.split_once(' ').unwrap_or((line, "0"));
        match op {
            "group-cases" => {
                c.execute_batch("BEGIN IMMEDIATE; ALTER TABLE Name2Id ADD COLUMN username TEXT; UPDATE Name2Id SET username=user_name; ALTER TABLE contact ADD COLUMN id INTEGER; CREATE TABLE chat_room(id INTEGER,username TEXT,owner TEXT); CREATE TABLE chatroom_member(room_id INTEGER,member_id INTEGER); INSERT INTO Name2Id(rowid,username) VALUES(501,'owned-group@chatroom'),(502,'other-group@chatroom'); INSERT INTO chat_room VALUES(501,'owned-group@chatroom','owned-member-1'),(502,'other-group@chatroom',NULL);").unwrap();
                for n in 1..=201i64 {
                    if n != 11 {
                        c.execute(
                            "INSERT INTO Name2Id(rowid,username) VALUES(?1,?2)",
                            rusqlite::params![1000 + n, format!("owned-member-{n}")],
                        )
                        .unwrap();
                    }
                    c.execute("INSERT INTO contact(id,username,nick_name,remark) VALUES(?1,?2,'群成员昵称','自动化测试')",rusqlite::params![1000+n,format!("owned-member-{n}")]).unwrap();
                    c.execute(
                        "INSERT INTO chatroom_member(rowid,room_id,member_id) VALUES(?1,501,?2)",
                        rusqlite::params![9007199254740992i64 + n, 1000 + n],
                    )
                    .unwrap();
                }
                c.execute_batch("UPDATE Name2Id SET username=CAST(X'FF' AS TEXT) WHERE rowid=1010; UPDATE contact SET nick_name=replace(hex(zeroblob(1025)),'00','x') WHERE id=1012; INSERT INTO chatroom_member(room_id,member_id) VALUES(502,1001); COMMIT;").unwrap();
                emit(json!({"group_cases":true}));
            }
            "sender-cases" => {
                use md5::{Digest, Md5};
                c.execute_batch("BEGIN IMMEDIATE; INSERT INTO Name2Id(rowid,user_name) VALUES(101,'owned-sender-one'),(102,'owned-发送者'),(103,''),(104,'bad'||char(0)||'id'),(105,replace(hex(zeroblob(257)),'00','x')),(106,X'6162'),(107,CAST(X'FF' AS TEXT)),(108,replace(hex(zeroblob(256)),'00','x'));").unwrap();
                for table in [
                    live_protocol::FILEHELPER_TABLE.to_owned(),
                    format!("Msg_{:x}", Md5::digest(b"owned-\xe4\xbc\x9a\xe8\xaf\x9d")),
                ] {
                    c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN real_sender_id; UPDATE {table} SET real_sender_id=CASE local_id%14 WHEN 0 THEN 101 WHEN 1 THEN 102 WHEN 2 THEN 0 WHEN 3 THEN NULL WHEN 4 THEN 999 WHEN 5 THEN 103 WHEN 6 THEN 104 WHEN 7 THEN 105 WHEN 8 THEN 106 WHEN 9 THEN 107 WHEN 10 THEN '101' WHEN 11 THEN 101.5 WHEN 12 THEN -1 WHEN 13 THEN 108 END;")).unwrap();
                }
                c.execute_batch("COMMIT").unwrap();
                emit(json!({"sender_cases":true}));
            }
            "conversation-cases" => {
                use md5::{Digest, Md5};
                c.execute_batch("BEGIN IMMEDIATE;").unwrap();
                for (username, body) in [
                    ("owned-会话", "owned-conversation-first"),
                    ("owned-'sql;--", "owned-conversation-second"),
                ] {
                    c.execute("INSERT INTO Name2Id(user_name) VALUES(?1)", [username])
                        .unwrap();
                    let table = format!("Msg_{:x}", Md5::digest(username.as_bytes()));
                    c.execute_batch(&format!("CREATE TABLE {table}(local_id INTEGER PRIMARY KEY, create_time INTEGER NOT NULL, local_type INTEGER NOT NULL, server_id INTEGER, message_content, compress_content)")).unwrap();
                    for id in 1..=125 {
                        c.execute(
                            &format!("INSERT INTO {table} VALUES(?1,1700000000,1,?2,?3,NULL)"),
                            rusqlite::params![id, 9007199254740992i64 + id, body],
                        )
                        .unwrap();
                    }
                }
                c.execute_batch("COMMIT").unwrap();
                emit(json!({"conversation_cases":true}));
            }
            "moment-cases" => {
                c.execute_batch("CREATE TABLE SnsTimeLine(tid INTEGER,user_name TEXT,content TEXT,pack_info_buf TEXT); BEGIN IMMEDIATE;").unwrap();
                for n in 1..=31u64 {
                    let id = u64::MAX - 32 + n;
                    let xml = format!(
                        "<SnsDataItem><TimelineObject><id>{id}</id><username>owned-moment-{n}</username><createTime>1700000000</createTime><contentDesc>自动化测试 &amp; 自有朋友圈</contentDesc></TimelineObject></SnsDataItem>"
                    );
                    c.execute("INSERT INTO SnsTimeLine(rowid,tid,user_name,content,pack_info_buf) VALUES(?1,?2,?3,?4,'secret')",rusqlite::params![9007199254740992i64+n as i64,id as i64,format!("owned-moment-{n}"),xml]).unwrap();
                }
                c.execute_batch("COMMIT").unwrap();
                emit(json!({"moment_cases":true}));
            }
            "session-cases" => {
                c.execute_batch("CREATE TABLE SessionTable(username TEXT,sort_timestamp INTEGER,last_timestamp INTEGER,summary TEXT,unread_count INTEGER,is_hidden INTEGER,draft TEXT); BEGIN IMMEDIATE;").unwrap();
                for n in 1..=201 {
                    c.execute("INSERT INTO SessionTable(rowid,username,sort_timestamp,last_timestamp,summary,unread_count,is_hidden,draft) VALUES(?1,?2,20,19,'自有摘要',0,0,'private draft excluded')",rusqlite::params![9007199254740992i64+n,format!("owned-session-{n}")]).unwrap();
                }
                c.execute_batch("COMMIT").unwrap();
                emit(json!({"session_cases":true}));
            }
            "contact-cases" => {
                c.execute_batch("CREATE TABLE contact(username TEXT,nick_name TEXT,remark TEXT,alias TEXT,local_type INTEGER,delete_flag INTEGER,verify_flag INTEGER); BEGIN IMMEDIATE;").unwrap();
                for n in 1..=201 {
                    c.execute("INSERT INTO contact(rowid,username,nick_name,remark,alias,local_type,delete_flag,verify_flag) VALUES(?1,?2,'自有昵称','自动化测试',NULL,0,0,0)", rusqlite::params![9007199254740992i64+n,format!("owned-contact-{n}")]).unwrap();
                }
                c.execute_batch("COMMIT").unwrap();
                emit(json!({"contact_cases":true}));
            }
            "insert" => {
                let n: i64 = amount.parse().unwrap();
                assert!((1..=5000).contains(&n));
                c.execute_batch("BEGIN IMMEDIATE;").unwrap();
                let mut q = c
                    .prepare("INSERT INTO fixture_messages(timestamp,body) VALUES(1700000000,?1)")
                    .unwrap();
                for _ in 0..n {
                    q.execute(["owned-synthetic-message".repeat(20)]).unwrap();
                }
                c.execute_batch("INSERT INTO Msg_9e20f478899dc29eb19741386f9343c8(local_id,create_time,local_type,server_id,message_content) SELECT id,timestamp,1,9007199254740992+id,body FROM fixture_messages WHERE id NOT IN (SELECT local_id FROM Msg_9e20f478899dc29eb19741386f9343c8)").unwrap();
                emit(json!({"uncommitted":n}));
            }
            "commit" => {
                c.execute_batch("COMMIT").unwrap();
                emit(stats(&c).unwrap());
            }
            "rollback" => {
                c.execute_batch("ROLLBACK").unwrap();
                emit(stats(&c).unwrap());
            }
            "checkpoint" => {
                let result: (i64, i64, i64) = c
                    .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })
                    .unwrap();
                emit(json!({"checkpoint":result}));
            }
            "contact-location-card-cases" => {
                c.execute_batch(
                    r#"BEGIN IMMEDIATE; INSERT INTO Msg_9e20f478899dc29eb19741386f9343c8 VALUES
                (3031,1700000005,42,1,'<msg username="owned-card" nickname="Test"/>',NULL),
                (3032,1700000005,48,2,'<msg><location x="31" y="121" label="Test"/></msg>',NULL),
                (3033,1700000005,25769803818,3,NULL,X'28B52FFD2003190000616263'),
                (3034,1700000005,25769803824,4,NULL,X'28B52FFD2003190000616263'); COMMIT;"#,
                )
                .unwrap();
                emit(json!({"contact_location_card_cases":4}));
            }
            "image-video-cases" => {
                c.execute_batch("BEGIN IMMEDIATE; INSERT INTO Msg_9e20f478899dc29eb19741386f9343c8 VALUES (3021,1700000004,3,1,'<msg><img/></msg>',NULL),(3022,1700000004,43,2,'<msg><videomsg/></msg>',NULL),(3023,1700000004,25769803779,3,NULL,X'28B52FFD2003190000616263'),(3024,1700000004,25769803819,4,NULL,X'28B52FFD2003190000616263'); COMMIT;").unwrap();
                emit(json!({"image_video_cases":4}));
            }
            "voice-cases" => {
                c.execute_batch("CREATE TABLE Name2Id(user_name TEXT); INSERT INTO Name2Id VALUES('filehelper'),('other'); CREATE TABLE VoiceInfo(chat_name_id INTEGER,create_time INTEGER,local_id INTEGER,svr_id INTEGER,voice_data BLOB,data_index TEXT); BEGIN IMMEDIATE;").unwrap();
                let bytes = b"\x02#!SILK_V3\x01\x00\x03";
                for (chat, time, server) in [
                    (1, 1700000000, 9223372036854775000_i64),
                    (2, 1700000000, 42),
                    (1, 1700000001, 42),
                    (1, 1700000000, 43),
                    (1, 1700000000, 43),
                ] {
                    c.execute(
                        "INSERT INTO VoiceInfo VALUES(?1,?2,7,?3,?4,'')",
                        rusqlite::params![chat, time, server, bytes.as_slice()],
                    )
                    .unwrap();
                }
                c.execute_batch("INSERT INTO VoiceInfo VALUES(1,1700000000,7,44,zeroblob(262145),''),(1,1700000000,7,45,'text',''),(1,1700000000,7,46,NULL,''); COMMIT;").unwrap();
                emit(json!({"voice_cases":true}));
            }
            "emoticon-cases" => {
                c.execute_batch("BEGIN IMMEDIATE; INSERT INTO Msg_9e20f478899dc29eb19741386f9343c8 VALUES (3011,1700000003,25769803823,1,'<msg><emoji md5=\"df4813390984d6155aabb73ea22dade0\"/></msg>',NULL),(3012,1700000003,47,2,NULL,X'28B52FFD2003190000616263'); COMMIT;").unwrap();
                emit(json!({"emoticon_cases":2}));
            }
            "file-cases" => {
                c.execute_batch("BEGIN IMMEDIATE; INSERT INTO Msg_9e20f478899dc29eb19741386f9343c8 VALUES (3001,1700000002,25769803825,1,'<msg><appmsg><type>6</type></appmsg></msg>',NULL),(3002,1700000002,49,2,NULL,X'28B52FFD2003190000616263'); COMMIT;").unwrap();
                emit(json!({"file_cases":2}));
            }
            "text-cases" => {
                c.execute_batch(
                    "BEGIN IMMEDIATE;
                    INSERT INTO Msg_9e20f478899dc29eb19741386f9343c8 VALUES
                    (2001,1700000001,1,1,X'28B52FFD2003190000616263',NULL),
                    (2002,1700000001,1,2,99,X'28B52FFD2003190000616263'),
                    (2003,1700000001,1,3,'must not fall back',X'28B52FFD20'),
                    (2005,1700000001,1,5,NULL,NULL),
                    (2006,1700000001,3,6,'unsupported media',NULL),
                    (2007,1700000001,1,7,X'28B52FFD00F8',NULL),
                    (2008,1700000001,1,8,'owned plain','');",
                )
                .unwrap();
                c.execute("INSERT INTO Msg_9e20f478899dc29eb19741386f9343c8 VALUES(2004,1700000001,1,4,?1,NULL)",["x".repeat(16385)]).unwrap();
                c.execute_batch("COMMIT").unwrap();
                emit(json!({"text_cases":8}));
            }
            "end" => {
                emit(json!({"closed":true}));
                return false;
            }
            _ => panic!("unknown fixture command"),
        }
        true
    });
}

fn reader(path: &Path, held: bool, wrong_key: bool) -> Result<(), &'static str> {
    let key = if wrong_key { "6b" } else { "5a" }.repeat(32);
    let metadata = live::probe(path, &key)?;
    assert_eq!(metadata["read_only"], true);
    let view = live::ReadView::open(path, &key)?;
    let c = &view.connection;
    let first = stats(c).map_err(|_| "FIXTURE_READ_FAILED")?;
    let blocked = c
        .execute("UPDATE fixture_messages SET timestamp=0", [])
        .unwrap_err();
    assert_eq!(
        blocked.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ReadOnly)
    );
    emit(json!({"snapshot":first,"write_rejected":true}));
    if held {
        commands(|line| match line {
            "read" => {
                emit(json!({"snapshot":stats(c).unwrap()}));
                true
            }
            "end" => false,
            _ => panic!("unknown reader command"),
        });
    }
    c.execute_batch("ROLLBACK").map_err(|_| "ROLLBACK_FAILED")?;
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    assert_eq!(args.len(), 3);
    let path = Path::new(&args[2]);
    validate(path);
    let mode = args[1].to_str().unwrap();
    if mode == "writer" {
        writer(path);
        return;
    }
    if mode == "window" {
        let window = window::read(path, &"5a".repeat(32), 1200, None).unwrap();
        assert!(window.has_older);
        assert_eq!(window.rows.len(), 1200);
        assert!(
            window
                .rows
                .windows(2)
                .all(|r| r[0].local_id.parse::<i64>().unwrap()
                    == r[1].local_id.parse::<i64>().unwrap() + 1)
        );
        assert!(window.rows.iter().all(|r| r.text.as_deref()
            == Some("owned-synthetic-message".repeat(20).as_str())
            && r.server_id.as_ref().unwrap().parse::<i64>().unwrap() > 9007199254740991));
        emit(json!({"window_verified":true,"rows":window.rows.len(),"has_older":window.has_older}));
        return;
    }
    if mode == "deadline" {
        let view = live::ReadView::open(path, &"5a".repeat(32)).unwrap();
        let started = std::time::Instant::now();
        let result=view.connection.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",[],|r|r.get::<_,i64>(0));
        assert_eq!(
            result.unwrap_err().sqlite_error_code(),
            Some(rusqlite::ErrorCode::OperationInterrupted)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        emit(json!({"deadline_interrupted":true}));
        return;
    }
    assert!(["reader", "held-reader", "wrong-key"].contains(&mode));
    match reader(path, mode == "held-reader", mode == "wrong-key") {
        Ok(()) => (),
        Err(_) => {
            emit(json!({"read_failed":true}));
            std::process::exit(3);
        }
    }
}
