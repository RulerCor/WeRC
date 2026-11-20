#![cfg(windows)]
use std::sync::Arc;
use werc_application::{Application, ports::CapabilityProvider};
use werc_domain::{AppResult, browse::*, history::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

struct Provider;
impl CapabilityProvider for Provider {
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        vec![]
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.unprotect_key(c, k)
    }
    fn read_database(&self, r: &HistoryReadRequest) -> AppResult<HistoryBundle> {
        let second = r.path == "second";
        Ok(HistoryBundle {
            schema: "synthetic".into(),
            source_sha256: if second { "2" } else { "1" }.repeat(64),
            encryption: "synthetic".into(),
            wal_committed_frames: 0,
            messages: (0..12)
                .map(|i| HistoryMessage {
                    id: format!("synthetic-{i}"),
                    conversation_id: "synthetic-peer".into(),
                    sender_id: Some("synthetic-peer".into()),
                    local_id: i,
                    server_id: (i64::MAX - 12 + i).to_string(),
                    timestamp: 1700000000 + i / 4,
                    sequence: i64::MAX - 20 + i,
                    message_type: 1,
                    text: Some(if second && i == 11 {
                        "updated synthetic message".into()
                    } else {
                        format!("<script>synthetic-{i}</script>")
                    }),
                    content_status: "decoded".into(),
                })
                .collect(),
            contacts: vec![HistoryContact {
                username: "synthetic-peer".into(),
                nickname: Some("<img src=x onerror=alert(1)>".into()),
                remark: None,
                alias: None,
            }],
            sessions: vec![HistorySession {
                username: "synthetic-peer".into(),
                summary: Some("synthetic".into()),
                timestamp: 1699999999,
                unread_count: Some(7),
            }],
            warnings: vec![],
        })
    }
}
struct BlockingProvider {
    enabled: std::sync::atomic::AtomicBool,
    gate: std::sync::Mutex<(usize, bool)>,
    wake: std::sync::Condvar,
}
impl BlockingProvider {
    fn release(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.1 = true;
        self.wake.notify_all();
    }
}
impl CapabilityProvider for BlockingProvider {
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        vec![]
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        Provider.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        if self.enabled.load(std::sync::atomic::Ordering::SeqCst) {
            let mut gate = self.gate.lock().unwrap();
            gate.0 += 1;
            self.wake.notify_all();
            while !gate.1 {
                gate = self.wake.wait(gate).unwrap();
            }
        }
        Provider.unprotect_key(c, k)
    }
    fn read_database(&self, r: &HistoryReadRequest) -> AppResult<HistoryBundle> {
        Provider.read_database(r)
    }
}
struct ReleaseGate(Arc<BlockingProvider>);
impl Drop for ReleaseGate {
    fn drop(&mut self) {
        self.0.release();
    }
}
#[test]
fn replacing_a_canceled_projection_waits_and_the_wait_remains_cancelable() {
    use std::{sync::mpsc, time::Duration};
    let provider = Arc::new(BlockingProvider {
        enabled: std::sync::atomic::AtomicBool::new(false),
        gate: std::sync::Mutex::new((0, false)),
        wake: std::sync::Condvar::new(),
    });
    let app = Arc::new(Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        provider.clone(),
    ));
    let account = app
        .register_account_local("owned projection switch")
        .unwrap()
        .id;
    app.import_history_local(
        &account,
        HistoryReadRequest {
            path: "first".into(),
            key_hex: None,
            allow_plaintext: true,
        },
        true,
    )
    .unwrap();
    provider
        .enabled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let _release = ReleaseGate(provider.clone());
    let spawn = |id: &str| {
        let app = app.clone();
        let account = account.clone();
        let mut request = query();
        request.request_id = id.into();
        let (send, receive) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = app.browse_messages_local(&account, &request);
            let _ = send.send(result);
        });
        (thread, receive)
    };
    let (first, first_result) = spawn("owned-projection-first");
    {
        let guard = provider.gate.lock().unwrap();
        let (guard, timeout) = provider
            .wake
            .wait_timeout_while(guard, Duration::from_secs(5), |g| g.0 == 0)
            .unwrap();
        assert!(!timeout.timed_out() && guard.0 > 0);
    }
    let (waiting, waiting_result) = spawn("owned-projection-waiting");
    assert!(matches!(
        waiting_result.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    app.cancel_browse_local("owned-projection-waiting").unwrap();
    assert_eq!(
        waiting_result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    waiting.join().unwrap();
    app.cancel_browse_local("owned-projection-first").unwrap();
    let (replacement, replacement_result) = spawn("owned-projection-replacement");
    assert!(matches!(
        replacement_result.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    provider.release();
    assert_eq!(
        first_result
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert!(
        !replacement_result
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .data
            .is_empty()
    );
    first.join().unwrap();
    replacement.join().unwrap();
}
#[test]
fn cancellation_keeps_both_running_slots_until_workers_really_exit() {
    let provider = Arc::new(BlockingProvider {
        enabled: std::sync::atomic::AtomicBool::new(false),
        gate: std::sync::Mutex::new((0, false)),
        wake: std::sync::Condvar::new(),
    });
    let app = Arc::new(Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        provider.clone(),
    ));
    let account = app
        .register_account_local("synthetic concurrency")
        .unwrap()
        .id;
    app.import_history_local(
        &account,
        HistoryReadRequest {
            path: "first".into(),
            key_hex: None,
            allow_plaintext: true,
        },
        true,
    )
    .unwrap();
    // 先准备投影；本测试验证两个热查询的取消额度，不与单构建锁竞争。
    app.browse_messages_local(&account, &query()).unwrap();
    provider
        .enabled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let _release = ReleaseGate(provider.clone());
    let requests = [query(), query()];
    let workers: Vec<_> = requests
        .iter()
        .cloned()
        .map(|request| {
            let app = app.clone();
            let account = account.clone();
            std::thread::spawn(move || app.browse_messages_local(&account, &request))
        })
        .collect();
    let gate = provider.gate.lock().unwrap();
    let (gate, timeout) = provider
        .wake
        .wait_timeout_while(gate, std::time::Duration::from_secs(5), |gate| gate.0 < 2)
        .unwrap();
    drop(gate);
    assert!(!timeout.timed_out());
    for request in requests {
        app.cancel_browse_local(&request.request_id).unwrap();
    }
    assert_eq!(
        app.browse_messages_local(&account, &query())
            .unwrap_err()
            .code,
        "BUSY"
    );
    provider.release();
    for worker in workers {
        assert_eq!(worker.join().unwrap().unwrap_err().code, "QUERY_CANCELED");
    }
    assert!(app.browse_messages_local(&account, &query()).is_ok());
}

fn query() -> BrowseRequest {
    BrowseRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        mode: ReadMode::ImportedSnapshot,
        source_id: None,
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
            limit: 3,
        },
    }
}
fn fixture() -> (Application, String, HistorySource) {
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(Provider),
    );
    let account = app.register_account_local("合成聊天").unwrap().id;
    let source = app
        .import_history_local(
            &account,
            HistoryReadRequest {
                path: "first".into(),
                key_hex: None,
                allow_plaintext: true,
            },
            true,
        )
        .unwrap();
    (app, account, source)
}
async fn post(
    app: Arc<Application>,
    secret: &str,
    path: &str,
    input: serde_json::Value,
) -> (axum::http::StatusCode, serde_json::Value) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let response = werc_http_api::router(app)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("Authorization", format!("Bearer {secret}"))
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(input.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}
#[tokio::test]
async fn api_chat_scope_cancel_namespace_and_revocation_are_enforced() {
    use axum::http::StatusCode;
    let (app, account, _) = fixture();
    let app = Arc::new(app);
    let scopes = vec![
        "messages:read".into(),
        "contacts:read".into(),
        "sessions:read".into(),
    ];
    let a = app
        .issue_token_local("synthetic-A", vec![account.clone()], scopes.clone(), 1)
        .unwrap();
    let b = app
        .issue_token_local("synthetic-B", vec![account.clone()], scopes, 1)
        .unwrap();
    let limited = app
        .issue_token_local(
            "synthetic-limited",
            vec![account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let request = query();
    let input = serde_json::json!({"account_id":account,"request":request});
    assert_eq!(
        post(
            app.clone(),
            &limited.secret,
            "/api/v1/browse/messages",
            input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            app.clone(),
            &a.secret,
            "/api/v1/browse/messages",
            serde_json::json!({"account_id":"synthetic-foreign","request":request})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let cancel = serde_json::json!({"account_id":account,"request_id":request.request_id});
    assert_eq!(
        post(app.clone(), &a.secret, "/api/v1/browse/cancel", cancel)
            .await
            .0,
        StatusCode::OK
    );
    let (status, page) = post(
        app.clone(),
        &b.secret,
        "/api/v1/browse/messages",
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["context"]["mode"], "imported_snapshot");
    assert!(page["data"][0]["sequence"].is_string());
    let (status, error) = post(
        app.clone(),
        &a.secret,
        "/api/v1/browse/messages",
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"]["code"], "QUERY_CANCELED");
    app.revoke_token_local(&b.info.id).unwrap();
    assert_eq!(
        post(app, &b.secret, "/api/v1/browse/messages", input)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn latest_older_newer_pages_preserve_64bit_ids_and_same_second_order() {
    let (app, account, _) = fixture();
    let latest = app.browse_messages_local(&account, &query()).unwrap();
    assert_eq!(latest.data.len(), 3);
    assert_eq!(latest.data[2].server_id, (i64::MAX - 1).to_string());
    assert_eq!(latest.data[2].sequence, (i64::MAX - 9).to_string());
    assert_eq!(latest.data[0].direction, "unknown");
    assert_eq!(latest.data[0].identity_status, "unverified");
    assert!(latest.newer_cursor.is_none());
    let mut request = query();
    request.direction = BrowseDirection::Older;
    request.query.cursor = latest.older_cursor.clone();
    let older = app.browse_messages_local(&account, &request).unwrap();
    assert_eq!(older.data.len(), 3);
    assert!(
        older
            .data
            .iter()
            .all(|a| latest.data.iter().all(|b| a.id != b.id))
    );
    request = query();
    request.direction = BrowseDirection::Newer;
    request.query.cursor = older.newer_cursor;
    let newer = app.browse_messages_local(&account, &request).unwrap();
    assert_eq!(
        newer.data.iter().map(|m| &m.id).collect::<Vec<_>>(),
        latest.data.iter().map(|m| &m.id).collect::<Vec<_>>()
    );
}
#[test]
fn authenticated_cursor_rejects_tamper_direction_filter_and_source_changes() {
    let (app, account, _) = fixture();
    let first = app.browse_messages_local(&account, &query()).unwrap();
    let mut older = query();
    older.direction = BrowseDirection::Older;
    older.query.cursor = first.older_cursor.clone();
    older.query.cursor.as_mut().unwrap().replace_range(..1, "z");
    assert_eq!(
        app.browse_messages_local(&account, &older)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
    older.query.cursor = first.older_cursor.clone();
    older.direction = BrowseDirection::Newer;
    assert_eq!(
        app.browse_messages_local(&account, &older)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
    older.direction = BrowseDirection::Older;
    older.query.search = Some("changed".into());
    assert_eq!(
        app.browse_messages_local(&account, &older)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
    older.query.search = None;
    app.import_history_local(
        &account,
        HistoryReadRequest {
            path: "second".into(),
            key_hex: None,
            allow_plaintext: true,
        },
        true,
    )
    .unwrap();
    assert_eq!(
        app.browse_messages_local(&account, &older)
            .unwrap_err()
            .code,
        "CURSOR_STALE"
    );
}
#[test]
fn individual_snapshot_keeps_its_original_revision_and_aggregate_uses_latest() {
    let (app, account, source) = fixture();
    app.import_history_local(
        &account,
        HistoryReadRequest {
            path: "second".into(),
            key_hex: None,
            allow_plaintext: true,
        },
        true,
    )
    .unwrap();
    let latest = app.browse_messages_local(&account, &query()).unwrap();
    assert_eq!(
        latest.data[2].text.as_deref(),
        Some("updated synthetic message")
    );
    let mut request = query();
    request.source_id = Some(source.id);
    let old = app.browse_messages_local(&account, &request).unwrap();
    assert_eq!(
        old.data[2].text.as_deref(),
        Some("<script>synthetic-11</script>")
    );
    assert_eq!(old.context.sources.len(), 1);
    assert_eq!(latest.context.sources.len(), 2);
}
#[test]
fn anchor_and_conversation_metadata_do_not_guess_login_identity_or_execute_markup() {
    let (app, account, _) = fixture();
    let latest = app.browse_messages_local(&account, &query()).unwrap();
    let mut request = query();
    request.direction = BrowseDirection::Around;
    request.anchor_id = Some(latest.data[0].id.clone());
    request.query.limit = 4;
    let located = app.browse_messages_local(&account, &request).unwrap();
    assert!(located.data.iter().any(|m| m.id == latest.data[0].id));
    assert_eq!(located.data.len(), 4);
    request.anchor_id = None;
    request.anchor_timestamp = Some(1700000001);
    assert!(
        !app.browse_messages_local(&account, &request)
            .unwrap()
            .data
            .is_empty()
    );
    request = query();
    request.direction = BrowseDirection::Around;
    request.anchor_timestamp = Some(0);
    request.query.limit = 5;
    let edge = app.browse_messages_local(&account, &request).unwrap();
    assert_eq!(edge.data.len(), 5);
    assert!(edge.older_cursor.is_none());
    assert!(edge.newer_cursor.is_some());
    request.anchor_timestamp = Some(i64::MAX);
    let edge = app.browse_messages_local(&account, &request).unwrap();
    assert_eq!(edge.data.len(), 5);
    assert!(edge.older_cursor.is_some());
    assert!(edge.newer_cursor.is_none());
    request = query();
    request.query.conversation_id = None;
    let conversations = app.browse_conversations_local(&account, &request).unwrap();
    assert_eq!(conversations.data.len(), 1);
    assert_eq!(
        conversations.data[0].display_name,
        "<img src=x onerror=alert(1)>"
    );
    assert_eq!(
        conversations.data[0].historical_unread_count.as_deref(),
        Some("7")
    );
    assert_eq!(
        conversations.context.query_strategy,
        "protected_batch_projection"
    );
    assert_eq!(conversations.context.identity_status, "unverified");
}
#[test]
fn online_modes_never_fall_back_and_cancel_before_start_does_no_work() {
    let (app, account, _) = fixture();
    let mut request = query();
    for mode in [ReadMode::Realtime, ReadMode::OnDemandHistory] {
        request.mode = mode;
        assert_eq!(
            app.browse_messages_local(&account, &request)
                .unwrap_err()
                .code,
            "CAPABILITY_NOT_IMPLEMENTED"
        );
    }
    request.mode = ReadMode::ImportedSnapshot;
    app.cancel_browse_local(&request.request_id).unwrap();
    assert_eq!(
        app.browse_messages_local(&account, &request)
            .unwrap_err()
            .code,
        "QUERY_CANCELED"
    );
    assert!(app.browse_messages_local(&account, &query()).is_ok());
}
