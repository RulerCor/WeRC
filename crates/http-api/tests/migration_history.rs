#![cfg(windows)]
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rusqlite::{Connection, params};
use std::sync::Arc;
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{browse::*, history::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

#[test]
fn legacy_encrypted_histories_keep_aad_and_remain_browsable_before_and_after_selection() {
    let directory =
        std::env::temp_dir().join(format!("werc-legacy-history-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("state.sqlite3");
    let old = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../../infrastructure/migrations/0001.sql"),
        include_str!("../../infrastructure/migrations/0002.sql"),
        include_str!("../../infrastructure/migrations/0003.sql"),
        include_str!("../../infrastructure/migrations/0004.sql"),
        include_str!("../../infrastructure/migrations/0005.sql"),
        include_str!("../../infrastructure/migrations/0006.sql"),
        include_str!("../../infrastructure/migrations/0007.sql"),
        include_str!("../../infrastructure/migrations/0008.sql"),
    ] {
        old.execute_batch(migration).unwrap();
    }
    let mut originals = Vec::new();
    for (index, account) in ["legacy-current", "legacy-archive"].into_iter().enumerate() {
        old.execute("INSERT INTO accounts VALUES(?1,?1,'2026-01-01')", [account])
            .unwrap();
        let source = HistorySource {
            id: format!("source-{index}"),
            account_id: account.into(),
            source_sha256: (index + 1).to_string().repeat(64),
            schema: "synthetic".into(),
            encryption: "synthetic".into(),
            message_count: 1,
            contact_count: 0,
            session_count: 0,
            imported_at: "2026-01-01T00:00:00Z".into(),
            real_wechat_verified: false,
        };
        let bundle = HistoryBundle {
            schema: "synthetic".into(),
            source_sha256: source.source_sha256.clone(),
            encryption: "synthetic".into(),
            wal_committed_frames: 0,
            messages: vec![HistoryMessage {
                id: "a".repeat(64),
                conversation_id: "synthetic-peer".into(),
                sender_id: None,
                local_id: 1,
                server_id: "9223372036854775806".into(),
                timestamp: 1700000000,
                sequence: 1,
                message_type: 1,
                text: Some(format!("synthetic-private-{account}")),
                content_status: "decoded".into(),
            }],
            contacts: vec![],
            sessions: vec![],
            warnings: vec![],
        };
        let key = [index as u8 + 41; 32];
        let nonce = [index as u8 + 9; 12];
        let wrapped = WindowsWechatProvider
            .protect_key(&format!("WeRC/history/v1/{account}/{}", source.id), &key)
            .unwrap();
        let aad = serde_json::to_vec(&source).unwrap();
        let ciphertext = Aes256Gcm::new_from_slice(&key)
            .unwrap()
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &serde_json::to_vec(&bundle).unwrap(),
                    aad: &aad,
                },
            )
            .unwrap();
        old.execute("INSERT INTO history_sources(id,account_id,source_sha256,metadata,wrapped_key,nonce,ciphertext) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![source.id,account,source.source_sha256,serde_json::to_string(&source).unwrap(),wrapped,nonce.as_slice(),ciphertext]).unwrap();
        originals.push((account.to_string(), source.id, wrapped, ciphertext));
    }
    drop(old);
    let repo = Arc::new(SqliteRepository::open(&path).unwrap());
    let app = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
    assert!(app.current_connection_local().unwrap().selection_required);
    for phase in 0..2 {
        for (account, id, wrapped, ciphertext) in &originals {
            let data = repo.history_payload(account, id).unwrap();
            assert_eq!(&data.wrapped_key, wrapped);
            assert_eq!(&data.ciphertext, ciphertext);
            assert_eq!(data.source.account_id, *account);
            let request = BrowseRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                mode: ReadMode::ImportedSnapshot,
                source_id: Some(id.clone()),
                direction: BrowseDirection::Latest,
                anchor_id: None,
                anchor_timestamp: None,
                query: HistoryQuery {
                    conversation_id: Some("synthetic-peer".into()),
                    sender_id: None,
                    message_type: None,
                    search: None,
                    from: None,
                    until: None,
                    cursor: None,
                    limit: 10,
                },
            };
            let page = app.browse_messages_local(account, &request).unwrap();
            assert_eq!(page.data.len(), 1);
            assert_eq!(
                page.data[0].text,
                Some(format!("synthetic-private-{account}"))
            );
        }
        if phase == 0 {
            app.select_current_connection_local("legacy-current", "0", true)
                .unwrap();
        }
    }
    assert_eq!(app.accounts_local().unwrap().len(), 2);
    assert_eq!(
        app.import_history_local(
            "legacy-archive",
            HistoryReadRequest {
                path: "never-opened".into(),
                key_hex: None,
                allow_plaintext: true
            },
            true
        )
        .unwrap_err()
        .code,
        "CURRENT_CONNECTION_REQUIRED"
    );
    drop(app);
    drop(repo);
    std::fs::remove_dir_all(directory).unwrap();
}
