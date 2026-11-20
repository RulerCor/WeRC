#![cfg(windows)]
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::{path::PathBuf, sync::Arc};
use tower::ServiceExt;
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppError, AppResult, history::*};
use werc_infrastructure::SqliteRepository;

struct FixtureProvider;
impl CapabilityProvider for FixtureProvider {
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        Vec::new()
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        werc_wechat_win::keys::protect(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        werc_wechat_win::keys::unprotect(c, k)
    }
    fn read_database(&self, r: &HistoryReadRequest) -> AppResult<HistoryBundle> {
        werc_data_reader::read_database(r).map_err(|c| AppError::new(c, "synthetic fixture reader"))
    }
}
struct Fixture {
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("{}.werc-fixture.sqlite3", uuid::Uuid::new_v4()));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let c = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .unwrap();
        c.execute_batch("PRAGMA page_size=4096; CREATE TABLE contact(username TEXT,nick_name TEXT); INSERT INTO contact VALUES('synthetic-peer','synthetic-contact'); CREATE TABLE SessionTable(username TEXT,last_timestamp INTEGER); INSERT INTO SessionTable VALUES('synthetic-peer',1700000000); CREATE TABLE Msg_0123456789abcdef0123456789abcdef(local_id INTEGER,create_time INTEGER,local_type INTEGER,message_content TEXT,server_id INTEGER); INSERT INTO Msg_0123456789abcdef0123456789abcdef VALUES(1,1700000000,1,'sensitive-history-contents',9223372036854775806),(2,1700000001,1,'=1+1',123);").unwrap();
        Self { path }
    }
    fn request(&self) -> HistoryReadRequest {
        HistoryReadRequest {
            path: self.path.to_str().unwrap().into(),
            key_hex: None,
            allow_plaintext: true,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn query() -> HistoryQuery {
    HistoryQuery {
        conversation_id: None,
        sender_id: None,
        message_type: None,
        search: None,
        from: None,
        until: None,
        cursor: None,
        limit: 1,
    }
}
fn setup() -> (Arc<SqliteRepository>, Arc<Application>, String) {
    let (repo, account, _) = common::legacy_pair("合成记录账号", "归档隔离资料");
    let app = Arc::new(Application::new(repo.clone(), Arc::new(FixtureProvider)));
    (repo, app, account.id)
}
#[test]
fn import_query_encrypt_cursor_changes_and_account_removal() {
    let fixture = Fixture::new();
    let (repo, app, account) = setup();
    assert_eq!(
        app.import_history_local(&account, fixture.request(), false)
            .unwrap_err()
            .code,
        "CONSENT_REQUIRED"
    );
    let original = std::fs::read(&fixture.path).unwrap();
    let source = app
        .import_history_local(&account, fixture.request(), true)
        .unwrap();
    assert!(!source.real_wechat_verified);
    assert_eq!(source.message_count, 2);
    assert_eq!(
        app.import_history_local(&account, fixture.request(), true)
            .unwrap()
            .id,
        source.id
    );
    let stored = repo.history_payload(&account, &source.id).unwrap();
    assert!(
        !stored
            .ciphertext
            .windows(b"sensitive-history-contents".len())
            .any(|w| w == b"sensitive-history-contents")
    );
    assert_eq!(app.history_contacts_local(&account).unwrap().len(), 1);
    let first = app.history_messages_local(&account, &query()).unwrap();
    assert_eq!(first.data.len(), 1);
    assert_eq!(first.data[0].server_id, "9223372036854775806");
    let mut next = query();
    next.cursor = first.next_cursor.clone();
    assert_eq!(
        app.history_messages_local(&account, &next).unwrap().data[0]
            .text
            .as_deref(),
        Some("=1+1")
    );
    let mut bad = query();
    bad.limit = 0;
    assert!(app.history_messages_local(&account, &bad).is_err());
    bad = query();
    bad.search = Some("not-present".into());
    assert!(
        app.history_messages_local(&account, &bad)
            .unwrap()
            .data
            .is_empty()
    );
    let csv = app.export_history_local(&account, &query(), "csv").unwrap();
    assert!(csv.contains("'=1+1"));
    let json = app
        .export_history_local(&account, &query(), "json")
        .unwrap();
    assert!(json.contains("imported_snapshot"));
    assert_eq!(std::fs::read(&fixture.path).unwrap(), original);
    let c = rusqlite::Connection::open(&fixture.path).unwrap();
    c.execute_batch("UPDATE Msg_0123456789abcdef0123456789abcdef SET message_content='synthetic-updated' WHERE local_id=1; INSERT INTO Msg_0123456789abcdef0123456789abcdef VALUES(3,1700000002,1,'synthetic-new',456);").unwrap();
    drop(c);
    app.import_history_local(&account, fixture.request(), true)
        .unwrap();
    assert_eq!(
        app.history_messages_local(&account, &next)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
    let mut q = query();
    q.limit = 100;
    let merged = app.history_messages_local(&account, &q).unwrap();
    assert_eq!(merged.data.len(), 3);
    assert_eq!(merged.data[0].text.as_deref(), Some("synthetic-updated"));
    let events = repo.events(0, &account, 200).unwrap();
    let changes: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "history.message_observed")
        .collect();
    assert_eq!(changes.len(), 4);
    assert_eq!(
        changes
            .iter()
            .filter(|e| e.payload["change"] == "updated")
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .all(|e| !e.payload.to_string().contains("sensitive-history-contents"))
    );
    let other = app
        .accounts_local()
        .unwrap()
        .into_iter()
        .find(|a| a.id == "fixture-archive")
        .unwrap();
    assert!(repo.history_payload(&other.id, &source.id).is_err());
    app.remove_account_local(&account).unwrap();
    assert!(repo.history_sources(&account).unwrap().is_empty());
}
#[tokio::test]
async fn actual_history_http_scopes_pagination_export_and_revocation() {
    let fixture = Fixture::new();
    let (_, app, account) = setup();
    app.import_history_local(&account, fixture.request(), true)
        .unwrap();
    let other = app
        .accounts_local()
        .unwrap()
        .into_iter()
        .find(|a| a.id == "fixture-archive")
        .unwrap();
    let token = app
        .issue_token_local(
            "history",
            vec![account.clone()],
            vec![
                "messages:read".into(),
                "contacts:read".into(),
                "sessions:read".into(),
                "sources:read".into(),
                "exports:create".into(),
            ],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let req = |path: String| {
        Request::builder()
            .uri(path)
            .header("authorization", format!("Bearer {}", token.secret))
            .body(Body::empty())
            .unwrap()
    };
    let response = router
        .clone()
        .oneshot(req(format!(
            "/api/v1/messages?account_id={account}&limit=1"
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(page["data_mode"], "imported_snapshot");
    assert!(page["next_cursor"].is_string());
    assert_eq!(
        router
            .clone()
            .oneshot(req(format!("/api/v1/messages?account_id={}", other.id)))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    for path in ["sources", "contacts", "sessions"] {
        assert_eq!(
            router
                .clone()
                .oneshot(req(format!("/api/v1/{path}?account_id={account}")))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    let exported = router
        .clone()
        .oneshot(req(format!(
            "/api/v1/exports/messages?account_id={account}&format=csv"
        )))
        .await
        .unwrap();
    assert_eq!(exported.status(), StatusCode::OK);
    assert!(
        exported.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment")
    );
    assert!(
        String::from_utf8(
            exported
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec()
        )
        .unwrap()
        .contains("'=1+1")
    );
    for format in ["html", "md", "jsonl", "sql"] {
        let response = router
            .clone()
            .oneshot(req(format!(
                "/api/v1/exports/messages?account_id={account}&format={format}"
            )))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .contains(&format!("messages.{format}"))
        );
        assert!(
            !response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
    }
    let filtered = router
        .clone()
        .oneshot(req(format!(
            "/api/v1/messages?account_id={account}&message_type=34"
        )))
        .await
        .unwrap();
    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered: serde_json::Value =
        serde_json::from_slice(&filtered.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(filtered["data"].as_array().unwrap().len(), 0);
    let readonly = app
        .issue_token_local(
            "no-export",
            vec![account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    let denied = Request::builder()
        .uri(format!("/api/v1/exports/messages?account_id={account}"))
        .header("authorization", format!("Bearer {}", readonly.secret))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(denied).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    app.revoke_token_local(&token.info.id).unwrap();
    assert_eq!(
        router
            .oneshot(req(format!("/api/v1/messages?account_id={account}")))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}
#[test]
fn persisted_history_reopens_authenticates_and_removes_only_the_import() {
    let fixture = Fixture::new();
    let state = Fixture {
        path: std::env::temp_dir().join(format!("{}.werc-state.sqlite3", uuid::Uuid::new_v4())),
    };
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&state.path)
        .unwrap();
    let repo = Arc::new(SqliteRepository::open(&state.path).unwrap());
    let app = Application::new(repo.clone(), Arc::new(FixtureProvider));
    let account = app.register_account_local("持久化合成账号").unwrap().id;
    let source = app
        .import_history_local(&account, fixture.request(), true)
        .unwrap();
    drop(app);
    drop(repo);
    let repo = Arc::new(SqliteRepository::open(&state.path).unwrap());
    let app = Application::new(repo.clone(), Arc::new(FixtureProvider));
    let mut filter = query();
    filter.from = Some(1700000001);
    filter.until = Some(1700000002);
    let page = app.history_messages_local(&account, &filter).unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].text.as_deref(), Some("=1+1"));
    filter.conversation_id = Some("no-such-conversation".into());
    assert!(
        app.history_messages_local(&account, &filter)
            .unwrap()
            .data
            .is_empty()
    );
    assert!(
        app.export_history_local(&account, &query(), "txt")
            .unwrap()
            .contains("sensitive-history-contents")
    );
    drop(app);
    drop(repo);
    let c = rusqlite::Connection::open(&state.path).unwrap();
    c.execute(
        "UPDATE history_sources SET ciphertext=zeroblob(length(ciphertext)) WHERE id=?1",
        [&source.id],
    )
    .unwrap();
    drop(c);
    let repo = Arc::new(SqliteRepository::open(&state.path).unwrap());
    let app = Application::new(repo.clone(), Arc::new(FixtureProvider));
    assert_eq!(
        app.history_messages_local(&account, &query())
            .unwrap_err()
            .code,
        "SOURCE_CORRUPTED"
    );
    let before = std::fs::read(&fixture.path).unwrap();
    app.remove_history_local(&account, &source.id).unwrap();
    assert!(repo.history_sources(&account).unwrap().is_empty());
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    assert_eq!(
        app.history_messages_local(&account, &query())
            .unwrap_err()
            .code,
        "CAPABILITY_NOT_IMPLEMENTED"
    );
}

#[test]
fn combined_filters_cursor_binding_and_sql_export_roundtrip() {
    let fixture = Fixture::new();
    let c = rusqlite::Connection::open(&fixture.path).unwrap();
    c.execute(
        "UPDATE Msg_0123456789abcdef0123456789abcdef SET message_content=?1 WHERE local_id=2",
        ["<script>\"quoted\"</script>\n');DROP TABLE nope;--\0tail"],
    )
    .unwrap();
    drop(c);
    let (_, app, account) = setup();
    app.import_history_local(&account, fixture.request(), true)
        .unwrap();
    let mut q = query();
    let initial = app.history_messages_local(&account, &q).unwrap();
    q.cursor = initial.next_cursor;
    q.sender_id = Some("missing".into());
    assert_eq!(
        app.history_messages_local(&account, &q).unwrap_err().code,
        "CURSOR_STALE"
    );
    q.cursor = None;
    assert!(
        app.history_messages_local(&account, &q)
            .unwrap()
            .data
            .is_empty()
    );
    q.sender_id = None;
    q.message_type = Some(34);
    assert!(
        app.history_messages_local(&account, &q)
            .unwrap()
            .data
            .is_empty()
    );
    q.message_type = Some(-1);
    assert_eq!(
        app.history_messages_local(&account, &q).unwrap_err().code,
        "INVALID_ARGUMENT"
    );
    q = query();
    q.message_type = Some(1);
    q.from = Some(1700000001);
    q.until = Some(1700000002);
    assert_eq!(
        app.history_messages_local(&account, &q).unwrap().data.len(),
        1
    );
    let sql = app.export_history_local(&account, &q, "sql").unwrap();
    let exported = rusqlite::Connection::open_in_memory().unwrap();
    exported.execute_batch(&sql).unwrap();
    let (server, text): (String, String) = exported
        .query_row("SELECT server_id,text FROM werc_export_messages", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(server, "123");
    assert!(text.ends_with("\0tail"));
    assert!(
        exported.execute_batch(&sql).is_err(),
        "existing export tables are not overwritten"
    );
    let html = app.export_history_local(&account, &q, "html").unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
#[ignore = "verify.ps1 builds and explicitly tests the separate reader executable"]
fn reader_image_pinning_and_actual_subprocess_protocol() {
    use sha2::Digest;
    let fixture = Fixture::new();
    let executable =
        PathBuf::from(std::env::var_os("WERC_READER_EXE").expect("explicit reader path required"));
    let hash = hex::encode(sha2::Sha256::digest(std::fs::read(&executable).unwrap()));
    let bundle = werc_wechat_win::reader::execute(&executable, &hash, &fixture.request()).unwrap();
    assert_eq!(bundle.messages.len(), 2);
    assert_eq!(
        werc_wechat_win::reader::execute(&executable, &"0".repeat(64), &fixture.request())
            .unwrap_err()
            .code,
        "COMPONENT_INTEGRITY_FAILED"
    );
    assert_eq!(
        werc_wechat_win::reader::execute(&executable, "", &fixture.request())
            .unwrap_err()
            .code,
        "READER_UNAVAILABLE"
    );
}
