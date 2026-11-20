use super::*;
use std::sync::{Arc, Barrier};

#[test]
fn v21_settings_upgrade_preserves_wal_and_creates_one_verified_backup() {
    let root = std::env::temp_dir().join(format!("werc-settings-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    drop(SqliteRepository::open(&path).unwrap());
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; PRAGMA user_version=21; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO accounts VALUES('owned-v21','settings WAL fixture','2026-11-14');").unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert_eq!(repo.accounts().unwrap()[0].label, "settings WAL fixture");
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(),
        22
    );
    assert!(repo.page_settings_changes("owned-v21", None, 20).unwrap().is_empty());
    let backups: Vec<_> = std::fs::read_dir(&root).unwrap().map(Result::unwrap)
        .filter(|p| p.file_name().to_string_lossy().contains("pre-v22-")).collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).unwrap();
    assert_eq!(
        backup.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(),
        21
    );
    assert_eq!(
        backup.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(),
        "ok"
    );
    assert_eq!(
        backup.query_row("SELECT label FROM accounts WHERE id='owned-v21'", [], |r| r.get::<_, String>(0)).unwrap(),
        "settings WAL fixture"
    );
    assert!(backup.prepare("SELECT * FROM wechat_settings_changes").is_err());
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(std::fs::read_dir(&root).unwrap().map(Result::unwrap)
        .filter(|p| p.file_name().to_string_lossy().contains("pre-v22-")).count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_task_check_is_not_truncated_by_completed_history() {
    let repo = SqliteRepository::in_memory().unwrap();
    let account = repo.register_account("owned-pending-history").unwrap().id;
    let other = "absent-owned-history".to_string();
    {
        let c = repo.connection().unwrap();
        let operation = werc_domain::operation::Operation {
            id:"old-pending".into(), account_id:account.clone(), creator_id:"owned".into(),
            kind:"audio.inspect".into(), asset_id:"owned".into(), request_digest:"owned".into(),
            state:werc_domain::operation::OperationState::Queued, created_at:Utc::now().to_rfc3339(),
            updated_at:Utc::now().to_rfc3339(), result:None,error_code:None,
        };
        for index in 0..202 {
            let mut row=operation.clone();row.id=format!("owned-task-{index}");
            if index>0 {row.state=werc_domain::operation::OperationState::Succeeded;}
            c.execute("INSERT INTO operations(id,account_id,creator_id,idempotency_key,state,record) VALUES(?1,?2,'owned',?1,?3,?4)",
                params![row.id,account,if index==0 {"queued"} else {"succeeded"},serde_json::to_string(&row).unwrap()]).unwrap();
        }
    }
    assert!(repo.operations(&account).unwrap().iter().all(|o|o.state==werc_domain::operation::OperationState::Succeeded));
    assert!(repo.has_pending_tasks(&account).unwrap());
    assert!(!repo.has_pending_tasks(&other).unwrap());
    assert_eq!(repo.remove_account(&account).unwrap_err().code,"ACCOUNT_BUSY");
    repo.connection().unwrap().execute("UPDATE operations SET state='canceled' WHERE account_id=?1 AND state='queued'",[&account]).unwrap();
    assert!(!repo.has_pending_tasks(&account).unwrap());
    repo.remove_account(&account).unwrap();
}

#[test]
fn v16_received_voice_upgrade_preserves_encrypted_recovery_and_creates_verified_backup() {
    let root = std::env::temp_dir().join(format!("werc-received-voice-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let repo = SqliteRepository::open(&path).unwrap();
    let account = repo.register_account("owned-v16").unwrap().id;
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; DROP TABLE received_video_assets; DROP TABLE received_file_assets; DROP TABLE received_voice_assets; PRAGMA user_version=16; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    old.execute("INSERT INTO receive_recovery_windows(account_id,id,summary,wrapped_key,nonce,ciphertext) VALUES(?1,'owned','opaque',x'0102',zeroblob(12),x'0304')",[&account]).unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert!(repo.files(&account).unwrap().is_empty());
    assert_eq!(
        old.query_row(
            "SELECT hex(ciphertext) FROM receive_recovery_windows",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "0304"
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v17-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        16
    );
    assert_eq!(
        backup
            .query_row(
                "SELECT hex(ciphertext) FROM receive_recovery_windows",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0304"
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains("pre-v17-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v15_received_image_upgrade_preserves_encrypted_recovery_and_creates_verified_backup() {
    let root = std::env::temp_dir().join(format!("werc-received-image-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let repo = SqliteRepository::open(&path).unwrap();
    let account = repo.register_account("owned-v15").unwrap().id;
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; DROP TABLE received_video_assets; DROP TABLE received_file_assets; DROP TABLE received_voice_assets; DROP TABLE received_image_assets; PRAGMA user_version=15; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    old.execute("INSERT INTO receive_recovery_windows(account_id,id,summary,wrapped_key,nonce,ciphertext) VALUES(?1,'owned','opaque',x'0102',zeroblob(12),x'0304')",[&account]).unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert!(repo.files(&account).unwrap().is_empty());
    assert_eq!(
        old.query_row(
            "SELECT hex(ciphertext) FROM receive_recovery_windows",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "0304"
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v16-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        15
    );
    assert_eq!(
        backup
            .query_row(
                "SELECT hex(ciphertext) FROM receive_recovery_windows",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0304"
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains("pre-v16-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v14_file_upgrade_preserves_encrypted_recovery_and_creates_verified_backup() {
    let root = std::env::temp_dir().join(format!("werc-file-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let repo = SqliteRepository::open(&path).unwrap();
    let account = repo.register_account("owned-v14").unwrap().id;
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; DROP TABLE received_video_assets; DROP TABLE received_file_assets; DROP TABLE received_voice_assets; DROP TABLE received_image_assets; DROP TABLE file_assets; PRAGMA user_version=14; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    old.execute("INSERT INTO receive_recovery_windows(account_id,id,summary,wrapped_key,nonce,ciphertext) VALUES(?1,'owned','opaque',x'0102',zeroblob(12),x'0304')",[&account]).unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert!(repo.files(&account).unwrap().is_empty());
    assert_eq!(
        old.query_row(
            "SELECT hex(ciphertext) FROM receive_recovery_windows",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "0304"
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v15-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        14
    );
    assert_eq!(
        backup
            .query_row(
                "SELECT hex(ciphertext) FROM receive_recovery_windows",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0304"
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains("pre-v15-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v13_image_upgrade_preserves_encrypted_recovery_and_creates_verified_backup() {
    let root = std::env::temp_dir().join(format!("werc-image-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let repo = SqliteRepository::open(&path).unwrap();
    let account = repo.register_account("owned-v13").unwrap().id;
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; DROP TABLE received_video_assets; DROP TABLE received_file_assets; DROP TABLE received_voice_assets; DROP TABLE received_image_assets; DROP TABLE file_assets; DROP TABLE image_assets; PRAGMA user_version=13; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    old.execute("INSERT INTO receive_recovery_windows(account_id,id,summary,wrapped_key,nonce,ciphertext) VALUES(?1,'owned','opaque',x'0102',zeroblob(12),x'0304')",[&account]).unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert!(repo.images(&account).unwrap().is_empty());
    assert_eq!(
        old.query_row(
            "SELECT hex(ciphertext) FROM receive_recovery_windows",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "0304"
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v14-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        13
    );
    assert_eq!(
        backup
            .query_row(
                "SELECT hex(ciphertext) FROM receive_recovery_windows",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0304"
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains("pre-v14-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v12_recovery_upgrade_backs_up_wal_and_preserves_receive() {
    let root = std::env::temp_dir().join(format!("werc-recovery-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let repo = SqliteRepository::open(&path).unwrap();
    let account = repo.register_account("owned-v12").unwrap().id;
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; DROP TABLE received_video_assets; DROP TABLE received_file_assets; DROP TABLE received_voice_assets; DROP TABLE received_image_assets; DROP TABLE file_assets; DROP TABLE image_assets; DROP TABLE receive_recovery_windows; PRAGMA user_version=12; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    old.execute(
        "INSERT INTO receive_streams(account_id,cursor) VALUES(?1,'owned-encrypted-cursor')",
        [&account],
    )
    .unwrap();
    old.execute("INSERT INTO received_texts(account_id,id,content_digest,wrapped_key,nonce,ciphertext) VALUES(?1,'owned-id','opaque',x'0102',zeroblob(12),x'0304')",[&account]).unwrap();
    let upgraded = SqliteRepository::open(&path).unwrap();
    assert_eq!(
        old.query_row("SELECT hex(ciphertext) FROM received_texts", [], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "0304"
    );
    assert_eq!(
        old.query_row("SELECT cursor FROM receive_streams", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "owned-encrypted-cursor"
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    assert!(upgraded.recovery_summaries(&account).unwrap().is_empty());
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v13-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        backup
            .query_row("SELECT hex(ciphertext) FROM received_texts", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "0304"
    );
    drop(backup);
    drop(upgraded);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains("pre-v13-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn legacy(count: usize) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for sql in [
        include_str!("../migrations/0001.sql"),
        include_str!("../migrations/0002.sql"),
        include_str!("../migrations/0003.sql"),
        include_str!("../migrations/0004.sql"),
        include_str!("../migrations/0005.sql"),
        include_str!("../migrations/0006.sql"),
        include_str!("../migrations/0007.sql"),
        include_str!("../migrations/0008.sql"),
    ] {
        c.execute_batch(sql).unwrap();
    }
    for i in 0..count {
        let id = format!("legacy-{i}");
        c.execute(
            "INSERT INTO accounts VALUES(?1,?2,'2026-01-01')",
            params![id, format!("旧资料{i}")],
        )
        .unwrap();
        c.execute("INSERT INTO protected_keys(account_id,protected,updated_at) VALUES(?1,?2,'2026-01-01')", params![id,b"synthetic-ciphertext-not-a-key".as_slice()]).unwrap();
    }
    c
}

#[test]
fn zero_and_one_legacy_registration_are_not_authenticated_identities() {
    let empty = SqliteRepository::initialize(legacy(0)).unwrap();
    assert_eq!(empty.current_connection().unwrap().account_id, None);
    let first = empty.register_account("唯一微信").unwrap();
    assert_eq!(
        empty.current_connection().unwrap().account_id.as_deref(),
        Some(first.id.as_str())
    );
    assert_eq!(
        empty.register_account("第二个").unwrap_err().code,
        "SINGLE_CONNECTION_REQUIRED"
    );
    assert_eq!(empty.setup_account("ignored").unwrap().id, first.id);
    let one = SqliteRepository::initialize(legacy(1)).unwrap();
    let slot = one.current_connection().unwrap();
    assert_eq!(slot.account_id.as_deref(), Some("legacy-0"));
    assert!(!slot.selection_required);
    assert!(!slot.identity_verified);
    assert_eq!(
        one.protected_key("legacy-0").unwrap().unwrap().0,
        b"synthetic-ciphertext-not-a-key"
    );
}

#[test]
fn multiple_legacy_scopes_require_once_only_selection_and_preserve_every_key() {
    let repo = SqliteRepository::initialize(legacy(3)).unwrap();
    let initial = repo.current_connection().unwrap();
    assert!(initial.selection_required);
    assert!(initial.account_id.is_none());
    assert_eq!(
        repo.setup_account("new").unwrap_err().code,
        "MIGRATION_SELECTION_REQUIRED"
    );
    assert_eq!(
        repo.select_current_connection("absent", &initial.revision)
            .unwrap_err()
            .code,
        "ACCOUNT_NOT_FOUND"
    );
    repo.select_current_connection("legacy-1", &initial.revision)
        .unwrap();
    assert_eq!(repo.accounts().unwrap().len(), 3);
    for i in 0..3 {
        assert_eq!(
            repo.protected_key(&format!("legacy-{i}"))
                .unwrap()
                .unwrap()
                .0,
            b"synthetic-ciphertext-not-a-key"
        );
    }
    assert_eq!(
        repo.select_current_connection("legacy-0", &initial.revision)
            .unwrap_err()
            .code,
        "CONNECTION_CHANGED"
    );
    assert_eq!(
        repo.select_current_connection("legacy-0", &repo.current_connection().unwrap().revision)
            .unwrap_err()
            .code,
        "SINGLE_CONNECTION_REQUIRED"
    );
    assert_eq!(
        repo.setup_account_for_source("scope", &"1".repeat(64))
            .unwrap()
            .id,
        "legacy-1"
    );
    assert_eq!(
        repo.setup_account_for_source("other", &"2".repeat(64))
            .unwrap_err()
            .code,
        "SOURCE_CONFLICT"
    );
}

#[test]
fn deleted_current_never_promotes_an_archive_or_reuses_old_identity() {
    let repo = SqliteRepository::initialize(legacy(2)).unwrap();
    repo.select_current_connection("legacy-0", "0").unwrap();
    repo.remove_account("legacy-0").unwrap();
    let next = repo.current_connection().unwrap();
    assert!(next.account_id.is_none());
    assert!(next.selection_required);
    assert_eq!(
        repo.register_account("new").unwrap_err().code,
        "MIGRATION_SELECTION_REQUIRED"
    );
    assert!(repo.protected_key("legacy-1").unwrap().is_some());
    repo.select_current_connection("legacy-1", &next.revision)
        .unwrap();
    assert_eq!(
        repo.current_connection().unwrap().account_id.as_deref(),
        Some("legacy-1")
    );
}

fn old_token(c: &Connection, id: &str, accounts: &[&str]) {
    let info = TokenInfo {
        id: id.into(),
        label: "synthetic token".into(),
        account_ids: accounts.iter().map(|s| (*s).into()).collect(),
        scopes: vec!["messages:read".into()],
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
        revoked: false,
    };
    c.execute(
        "INSERT INTO tokens(id,digest,info,expires_at,revoked) VALUES(?1,?1,?2,?3,0)",
        params![id, serde_json::to_string(&info).unwrap(), info.expires_at],
    )
    .unwrap();
    for account in accounts {
        c.execute(
            "INSERT INTO token_accounts VALUES(?1,?2)",
            params![id, account],
        )
        .unwrap();
    }
}

#[test]
fn migration_never_rebinds_or_expands_legacy_tokens() {
    let one = legacy(1);
    old_token(&one, "single-valid", &["legacy-0"]);
    old_token(&one, "empty-scope", &[]);
    let one = SqliteRepository::initialize(one).unwrap();
    assert_eq!(
        one.authenticate("single-valid").unwrap().account_ids,
        vec!["legacy-0"]
    );
    assert_eq!(
        one.authenticate("empty-scope").unwrap_err().code,
        "UNAUTHORIZED"
    );
    let two = legacy(2);
    old_token(&two, "old-single", &["legacy-0"]);
    old_token(&two, "old-multi", &["legacy-0", "legacy-1"]);
    let two = SqliteRepository::initialize(two).unwrap();
    for digest in ["old-single", "old-multi"] {
        assert_eq!(two.authenticate(digest).unwrap_err().code, "UNAUTHORIZED");
    }
    two.select_current_connection("legacy-0", "0").unwrap();
    for digest in ["old-single", "old-multi"] {
        assert_eq!(two.authenticate(digest).unwrap_err().code, "UNAUTHORIZED");
    }
    let tokens = two.tokens().unwrap();
    assert_eq!(tokens.len(), 2);
    assert!(tokens.iter().all(|t| t.revoked));
    assert_eq!(
        tokens
            .iter()
            .find(|t| t.id == "old-multi")
            .unwrap()
            .account_ids,
        vec!["legacy-0", "legacy-1"]
    );
    let info = TokenInfo {
        id: "new-multi".into(),
        label: "invalid".into(),
        account_ids: vec!["legacy-0".into(), "legacy-1".into()],
        scopes: vec!["messages:read".into()],
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
        revoked: false,
    };
    assert_eq!(
        two.create_token(&info, "new-multi").unwrap_err().code,
        "CURRENT_CONNECTION_REQUIRED"
    );
}

#[test]
fn separate_sqlite_connections_atomically_arbitrate_all_creation_entries() {
    let root = std::env::temp_dir().join(format!("werc-single-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let a = SqliteRepository::open(&path).unwrap();
    let b = SqliteRepository::open(&path).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let entered = barrier.clone();
    let writer = std::thread::spawn(move || {
        entered.wait();
        a.setup_account_for_source("source", &"3".repeat(64))
            .unwrap()
    });
    barrier.wait();
    let direct = b.register_account("manual");
    let source = writer.join().unwrap();
    if let Ok(direct) = direct {
        assert_eq!(direct.id, source.id);
    }
    assert_eq!(b.accounts().unwrap().len(), 1);
    assert_eq!(
        b.current_connection().unwrap().account_id.as_deref(),
        Some(source.id.as_str())
    );
    assert_eq!(b.setup_account("singleton").unwrap().id, source.id);
    drop(b);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn version_ten_upgrade_preserves_outbox_and_creates_verified_pre_v11_backup() {
    let root = std::env::temp_dir().join(format!("werc-v11-migration-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let old = legacy(1);
    old.execute_batch(include_str!("../migrations/0009.sql"))
        .unwrap();
    old.execute_batch(include_str!("../migrations/0010.sql"))
        .unwrap();
    old.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .unwrap();
    drop(old);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO send_outbox(id,account_id,creator_id,idempotency_digest,metadata,state,wrapped_key,nonce,ciphertext,updated_at) VALUES('synthetic-send','legacy-0','desktop','opaque','{}','unknown',x'0102',zeroblob(12),x'0304',1);").unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    let c = repo.connection().unwrap();
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let saved: (String, Vec<u8>) = c
        .query_row(
            "SELECT state,ciphertext FROM send_outbox WHERE id='synthetic-send'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, ("unknown".into(), vec![3, 4]));
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM history_projections", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(c);
    let backup = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.file_name().to_string_lossy().contains(".pre-v11-"))
        .unwrap();
    let backup =
        Connection::open_with_flags(backup.path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        10
    );
    assert_eq!(
        backup
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        backup
            .query_row("SELECT state FROM send_outbox", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "unknown"
    );
    drop(backup);
    drop(old);
    drop(repo);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn version_nine_upgrade_preserves_wal_keys_current_scope_and_verifiable_backup() {
    let root = std::env::temp_dir().join(format!("werc-v10-migration-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let v8 = legacy(1);
    v8.execute_batch(include_str!("../migrations/0009.sql"))
        .unwrap();
    v8.execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
        .unwrap();
    drop(v8);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; UPDATE accounts SET label='v9 WAL committed' WHERE id='legacy-0';").unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert_eq!(repo.accounts().unwrap()[0].label, "v9 WAL committed");
    assert_eq!(
        repo.current_connection().unwrap().account_id.as_deref(),
        Some("legacy-0")
    );
    assert_eq!(
        repo.protected_key("legacy-0").unwrap().unwrap().0,
        b"synthetic-ciphertext-not-a-key"
    );
    assert!(repo.sends("legacy-0").unwrap().is_empty());
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains(".pre-v10-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(
        backup
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        backup
            .query_row("SELECT label FROM accounts WHERE id='legacy-0'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "v9 WAL committed"
    );
    assert_eq!(
        repo.connection()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains(".pre-v10-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_upgrade_backs_up_committed_wal_and_reopens_without_another_backup() {
    let root = std::env::temp_dir().join(format!("werc-migration-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let old = Connection::open(&path).unwrap();
    old.execute_batch(include_str!("../migrations/0001.sql"))
        .unwrap();
    old.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO accounts VALUES('legacy','WAL committed','2026-01-01');").unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert_eq!(repo.accounts().unwrap()[0].id, "legacy");
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains(".pre-v9-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open(backups[0].path()).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        backup
            .query_row("SELECT label FROM accounts WHERE id='legacy'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "WAL committed"
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".pre-v9-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v17_received_file_upgrade_preserves_encrypted_recovery_and_creates_verified_backup() {
    let root = std::env::temp_dir().join(format!("werc-received-file-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let repo = SqliteRepository::open(&path).unwrap();
    let account = repo.register_account("owned-v17").unwrap().id;
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; DROP TABLE received_file_assets; DROP TABLE received_video_assets; PRAGMA user_version=17; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    old.execute("INSERT INTO receive_recovery_windows(account_id,id,summary,wrapped_key,nonce,ciphertext) VALUES(?1,'owned','opaque',x'0102',zeroblob(12),x'0304')",[&account]).unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert!(repo.files(&account).unwrap().is_empty());
    assert_eq!(
        old.query_row(
            "SELECT hex(ciphertext) FROM receive_recovery_windows",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "0304"
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v18-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        17
    );
    assert_eq!(
        backup
            .query_row(
                "SELECT hex(ciphertext) FROM receive_recovery_windows",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0304"
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains("pre-v18-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v18_received_video_upgrade_preserves_encrypted_recovery_and_creates_verified_backup() {
    let root = std::env::temp_dir().join(format!("werc-received-video-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let repo = SqliteRepository::open(&path).unwrap();
    let account = repo.register_account("owned-v18").unwrap().id;
    drop(repo);
    let old = Connection::open(&path).unwrap();
    old.execute_batch("DROP TABLE wechat_settings_changes; DROP TABLE received_video_assets; PRAGMA user_version=18; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    old.execute("INSERT INTO receive_recovery_windows(account_id,id,summary,wrapped_key,nonce,ciphertext) VALUES(?1,'owned','opaque',x'0102',zeroblob(12),x'0304')",[&account]).unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert!(repo.files(&account).unwrap().is_empty());
    assert_eq!(
        old.query_row(
            "SELECT hex(ciphertext) FROM receive_recovery_windows",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "0304"
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|e| e.file_name().to_string_lossy().contains("pre-v19-"))
        .collect();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        18
    );
    assert_eq!(
        backup
            .query_row(
                "SELECT hex(ciphertext) FROM receive_recovery_windows",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "0304"
    );
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|e| e.file_name().to_string_lossy().contains("pre-v19-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_v19_schema_upgrade_keeps_wal_records_and_creates_one_backup() {
    let root = std::env::temp_dir().join(format!("werc-v20-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let old = Connection::open(&path).unwrap();
    for sql in [
        include_str!("../migrations/0001.sql"),
        include_str!("../migrations/0002.sql"),
        include_str!("../migrations/0003.sql"),
        include_str!("../migrations/0004.sql"),
        include_str!("../migrations/0005.sql"),
        include_str!("../migrations/0006.sql"),
        include_str!("../migrations/0007.sql"),
        include_str!("../migrations/0008.sql"),
        include_str!("../migrations/0009.sql"),
        include_str!("../migrations/0010.sql"),
        include_str!("../migrations/0011.sql"),
        include_str!("../migrations/0012.sql"),
        include_str!("../migrations/0013.sql"),
        include_str!("../migrations/0014.sql"),
        include_str!("../migrations/0015.sql"),
        include_str!("../migrations/0016.sql"),
        include_str!("../migrations/0017.sql"),
        include_str!("../migrations/0018.sql"),
        include_str!("../migrations/0019.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO accounts VALUES('owned-v19','owned upgrade','2026-11-12');").unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert_eq!(repo.accounts().unwrap()[0].id, "owned-v19");
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|p| p.file_name().to_string_lossy().contains("pre-v20-"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        19
    );
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(backup.prepare("SELECT * FROM send_schedules").is_err());
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|p| p.file_name().to_string_lossy().contains("pre-v20-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v20_library_upgrade_preserves_committed_wal_and_creates_one_v21_backup() {
    let root = std::env::temp_dir().join(format!("werc-library-upgrade-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("state.sqlite3");
    let old = Connection::open(&path).unwrap();
    for sql in [
        include_str!("../migrations/0001.sql"),
        include_str!("../migrations/0002.sql"),
        include_str!("../migrations/0003.sql"),
        include_str!("../migrations/0004.sql"),
        include_str!("../migrations/0005.sql"),
        include_str!("../migrations/0006.sql"),
        include_str!("../migrations/0007.sql"),
        include_str!("../migrations/0008.sql"),
        include_str!("../migrations/0009.sql"),
        include_str!("../migrations/0010.sql"),
        include_str!("../migrations/0011.sql"),
        include_str!("../migrations/0012.sql"),
        include_str!("../migrations/0013.sql"),
        include_str!("../migrations/0014.sql"),
        include_str!("../migrations/0015.sql"),
        include_str!("../migrations/0016.sql"),
        include_str!("../migrations/0017.sql"),
        include_str!("../migrations/0018.sql"),
        include_str!("../migrations/0019.sql"),
        include_str!("../migrations/0020.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO accounts VALUES('owned-v20','automation test','2026-11-12');").unwrap();
    let repo = SqliteRepository::open(&path).unwrap();
    assert_eq!(repo.accounts().unwrap()[0].id, "owned-v20");
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    assert!(old.prepare("SELECT * FROM library_documents").is_ok());
    let backups = std::fs::read_dir(&root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|p| p.file_name().to_string_lossy().contains("pre-v21-"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let backup = Connection::open_with_flags(
        backups[0].path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        20
    );
    assert_eq!(
        backup
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(backup.prepare("SELECT * FROM library_documents").is_err());
    drop(backup);
    drop(repo);
    drop(old);
    drop(SqliteRepository::open(&path).unwrap());
    assert_eq!(
        std::fs::read_dir(&root)
            .unwrap()
            .map(Result::unwrap)
            .filter(|p| p.file_name().to_string_lossy().contains("pre-v21-"))
            .count(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
}
