use rusqlite::{Connection, OpenFlags, ffi};
use std::{fs::OpenOptions, path::PathBuf};
fn main() {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("new fixture directory required"),
    );
    std::fs::create_dir(&directory).expect("directory must not exist");
    let path = directory.join("plain.werc-fixture.sqlite3");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let c = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
    c.execute_batch("PRAGMA page_size=4096;").unwrap();
    let mut reserved: i32 = 80;
    let status = unsafe {
        ffi::sqlite3_file_control(
            c.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_RESERVE_BYTES,
            (&mut reserved as *mut i32).cast(),
        )
    };
    assert_eq!(status, ffi::SQLITE_OK);
    c.execute_batch("VACUUM; CREATE TABLE Name2Id(user_name TEXT); INSERT INTO Name2Id VALUES('synthetic-self'),('synthetic-peer'); CREATE TABLE contact(username TEXT,nick_name TEXT,remark TEXT,alias TEXT); INSERT INTO contact VALUES('synthetic-peer','合成联系人','测试备注',NULL); CREATE TABLE SessionTable(username TEXT,last_timestamp INTEGER,summary TEXT,unread_count INTEGER); INSERT INTO SessionTable VALUES('synthetic-peer',1700000000,'只用于测试',1);").unwrap();
    let table = format!("Msg_{:x}", md5::Md5::digest(b"synthetic-peer"));
    c.execute_batch(&format!("CREATE TABLE {table}(local_id INTEGER PRIMARY KEY,create_time INTEGER,local_type INTEGER,message_content TEXT,compress_content BLOB,server_id INTEGER,sort_seq INTEGER,real_sender_id INTEGER); INSERT INTO {table} VALUES(1,1700000000,1,'synthetic-before',NULL,9223372036854775806,1,2);")).unwrap();
    let raw = std::fs::read(&path).unwrap();
    assert_eq!(raw[20], 80);
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    c.execute(
        &format!("INSERT INTO {table} VALUES(2,1700000001,1,'synthetic-wal-only',NULL,123,2,1)"),
        [],
    )
    .unwrap();
    std::fs::copy(&path, directory.join("with-wal.werc-fixture.sqlite3")).unwrap();
    std::fs::copy(
        path.with_file_name("plain.werc-fixture.sqlite3-wal"),
        directory.join("with-wal.werc-fixture.sqlite3-wal"),
    )
    .unwrap();
    // 只关闭本工具创建的合成连接；不访问微信，不对用户来源执行checkpoint。
    drop(c);
    use sha2::Digest;
    println!("Synthetic plaintext and committed-WAL fixtures generated");
}
