use super::*;

struct ExactReader {
    inner: Arc<Reader>,
    mode: AtomicUsize,
    submissions: AtomicUsize,
    queries: AtomicUsize,
    message_type: std::sync::atomic::AtomicU64,
}
impl CapabilityProvider for ExactReader {
    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
    fn resolve_send_context(&self, a: &str, t: &str) -> AppResult<SendContext> {
        self.inner.resolve_send_context(a, t)
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        self.inner.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        self.inner.unprotect_key(c, k)
    }
    fn database_salt(&self, p: &str) -> AppResult<[u8; 16]> {
        self.inner.database_salt(p)
    }
    fn live_source_fingerprint(&self, p: &str) -> AppResult<String> {
        self.inner.live_source_fingerprint(p)
    }
    fn live_reader_fingerprint(&self) -> AppResult<String> {
        self.inner.live_reader_fingerprint()
    }
    fn validate_live_source(&self, a: &str, p: &str) -> AppResult<()> {
        self.inner.validate_live_source(a, p)
    }
    fn submit_text(&self, s: TextSubmission<'_>) -> AppResult<SendReceipt> {
        self.submissions.fetch_add(1, Ordering::SeqCst);
        Ok(SendReceipt {
            outbox_id: s.outbox_id.into(),
            attempt_id: s.attempt_id.into(),
            context_digest: s.context_digest.into(),
            evidence: SendEvidence::NativeQueued,
            provider_reference: Some("native-callback:1".into()),
        })
    }
    fn submit_file(&self, s: FileSubmission<'_>) -> AppResult<SendReceipt> {
        self.submit_text(s.submission)
    }
    fn reconcile_file(&self, s: TextReceiptQuery<'_>, r: Option<&str>) -> AppResult<SendReceipt> {
        self.reconcile_text(s, r)
    }
    fn reconcile_text(&self, s: TextReceiptQuery<'_>, _: Option<&str>) -> AppResult<SendReceipt> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        Ok(SendReceipt {
            outbox_id: s.outbox_id.into(),
            attempt_id: s.attempt_id.into(),
            context_digest: s.context_digest.into(),
            evidence: SendEvidence::NativeQueued,
            provider_reference: Some(
                NativeMessageIdentity {
                    local_id: 487,
                    timestamp: 1700000000,
                    message_type: self.message_type.load(Ordering::SeqCst),
                }
                .reference(),
            ),
        })
    }
    fn read_live_conversation(
        &self,
        _: &str,
        p: &str,
        _: &[u8],
        fingerprint: &str,
        s: &LiveConversationSelection,
        _: &AtomicBool,
    ) -> AppResult<LiveWindow> {
        werc_wechat_win::live_source::LiveSourcePin::open(std::path::Path::new(p))?
            .require(fingerprint)?;
        assert_eq!(s.username, "filehelper");
        assert_eq!(s.query.as_ref().unwrap().from_timestamp, Some(1700000000));
        let mode = self.mode.load(Ordering::SeqCst);
        if mode == 0 {
            return Err(AppError::new(
                "LIVE_READ_FAILED",
                "owned temporary read failure",
            ));
        }
        Ok(LiveWindow {
            schema_version: 1,
            has_older: false,
            read_only: true,
            readonly_shm: true,
            read_mode: "bounded_transaction_window".into(),
            rows: if mode == 6 {
                vec![]
            } else {
                vec![LiveMessage {
                    at_user_list: None,
                    local_id: if mode == 2 { "488" } else { "487" }.into(),
                    timestamp: 1700000000,
                    server_id: Some(
                        if mode == 3 {
                            "0"
                        } else {
                            "6828192415391867026"
                        }
                        .into(),
                    ),
                    message_type: self.message_type.load(Ordering::SeqCst).to_string(),
                    text: Some(
                        if mode == 4 {
                            "different body"
                        } else {
                            "自动化测试：精确回执"
                        }
                        .into(),
                    ),
                    metadata: None,
                    content_status: "text".into(),
                    sender_username: Some(if mode == 5 { "other" } else { "owned" }.into()),
                }]
            },
        })
    }
}
fn start() -> (Fixture, Arc<ExactReader>, Application, String) {
    let f = Fixture::new();
    f.reader.context.lock().unwrap().execution_mode = SendExecutionMode::Native;
    f.prepare().unwrap();
    let p = Arc::new(ExactReader {
        inner: f.reader.clone(),
        mode: AtomicUsize::new(0),
        submissions: AtomicUsize::new(0),
        queries: AtomicUsize::new(0),
        message_type: std::sync::atomic::AtomicU64::new(1),
    });
    let app = Application::new(f.repo.clone(), p.clone());
    let d = app
        .create_text_send_local(
            &f.account,
            "filehelper",
            "自动化测试：精确回执",
            "owned-exact-receipt-once",
        )
        .unwrap();
    app.approve_text_send_local(&f.account, &d.metadata.id, &d.metadata.request_digest, true)
        .unwrap();
    assert_eq!(
        app.execute_send_local(&f.account, &d.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert_eq!(
        app.reconcile_send_local(&f.account, &d.metadata.id)
            .unwrap_err()
            .code,
        "LIVE_READ_FAILED"
    );
    (f, p, app, d.metadata.id)
}
#[test]
fn exact_receipt_survives_read_failure_reopen_and_connection_change_without_resending() {
    let (f, p, app, id) = start();
    drop(app);
    assert!(
        f.repo
            .library_get(&f.account, &format!("send-exact-{id}"))
            .is_ok()
    );
    p.mode.store(1, Ordering::SeqCst);
    p.inner.context.lock().unwrap().epoch = "reconnected-owned".into();
    let reopened = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(reopened, p.clone());
    assert_eq!(
        app.reconcile_send_local(&f.account, &id).unwrap().state,
        SendState::ServerAccepted
    );
    assert!(app.execute_send_local(&f.account, &id).is_err());
    assert_eq!(p.submissions.load(Ordering::SeqCst), 1);
    assert_eq!(p.queries.load(Ordering::SeqCst), 1);
    // Deletion does not erase an already durable server acceptance.
    p.mode.store(6, Ordering::SeqCst);
    assert_eq!(
        app.reconcile_send_local(&f.account, &id).unwrap().state,
        SendState::ServerAccepted
    );
    let mut backup = Vec::new();
    assert_eq!(
        app.backup_library_local(
            &f.account,
            "owned-exact-backup-password",
            "owned-exact-backup",
            &mut backup
        )
        .unwrap(),
        0
    );
    let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
    assert!(!disk.windows(19).any(|b| b == b"6828192415391867026"));
}
#[test]
fn exact_receipt_never_uses_body_matching_or_another_sender_or_replaced_source() {
    let (f, p, app, id) = start();
    p.inner.context.lock().unwrap().epoch = "reconnected-owned".into();
    for mode in [2, 3, 6] {
        p.mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            app.reconcile_send_local(&f.account, &id).unwrap().state,
            SendState::Unknown
        );
    }
    for mode in [4, 5] {
        p.mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            app.reconcile_send_local(&f.account, &id).unwrap_err().code,
            "RECEIPT_MISMATCH"
        );
    }
    p.mode.store(1, Ordering::SeqCst);
    *p.inner.salt.lock().unwrap() = [0x99; 16];
    assert_eq!(
        app.reconcile_send_local(&f.account, &id).unwrap_err().code,
        "LIVE_SOURCE_REPLACED"
    );
    assert_eq!(p.submissions.load(Ordering::SeqCst), 1);
}
#[test]
fn exact_receipt_rejects_ambiguous_shards() {
    let (f, p, app, id) = start();
    // Add a second bound source before a separate task captures its proof.
    let second = f.database.with_file_name("message_1.db");
    std::fs::write(&second, vec![0; 4096]).unwrap();
    f.app
        .prepare_live_filehelper_local(
            &f.account,
            second.to_str().unwrap(),
            "owned-second-shard",
            true,
        )
        .unwrap();
    let d = app
        .create_text_send_local(
            &f.account,
            "filehelper",
            "自动化测试：精确回执",
            "owned-exact-second-attempt",
        )
        .unwrap();
    app.approve_text_send_local(&f.account, &d.metadata.id, &d.metadata.request_digest, true)
        .unwrap();
    app.execute_send_local(&f.account, &d.metadata.id).unwrap();
    p.mode.store(1, Ordering::SeqCst);
    assert_eq!(
        app.reconcile_send_local(&f.account, &d.metadata.id)
            .unwrap_err()
            .code,
        "RECEIPT_AMBIGUOUS"
    );
    assert_eq!(
        f.repo.send_status(&f.account, &id).unwrap().state,
        SendState::Unknown
    );
    assert_eq!(p.submissions.load(Ordering::SeqCst), 2);
}

#[test]
fn exact_local_record_is_separate_from_server_acceptance_and_survives_reopen() {
    let (f,p,app,id)=start();
    assert_eq!(app.send_preview_local(&f.account,&id).unwrap().local_record.unwrap().state,"unknown");
    p.mode.store(2,Ordering::SeqCst); // Same text at a different local ID is no evidence.
    app.reconcile_send_local(&f.account,&id).unwrap();
    assert_eq!(app.send_preview_local(&f.account,&id).unwrap().local_record.unwrap().state,"unknown");
    p.mode.store(3,Ordering::SeqCst); // Exact local row, server ID has not been assigned.
    assert_eq!(app.reconcile_send_local(&f.account,&id).unwrap().state,SendState::Unknown);
    let record=app.send_preview_local(&f.account,&id).unwrap().local_record.unwrap();
    assert_eq!(record.state,"written");assert!(record.observed_at.is_some());
    drop(app);
    let reopened=Application::new(Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap()),p.clone());
    p.mode.store(6,Ordering::SeqCst); // Later absence does not erase the historical observation.
    assert_eq!(reopened.reconcile_send_local(&f.account,&id).unwrap().state,SendState::Unknown);
    let saved=reopened.send_preview_local(&f.account,&id).unwrap().local_record.unwrap();
    assert_eq!(saved.state,"written");assert_eq!(saved.observed_at,record.observed_at);
    assert_eq!(p.submissions.load(Ordering::SeqCst),1);
}

#[tokio::test]
async fn exact_local_record_api_needs_scope_and_does_not_requery_or_expose_content() {
    use axum::{body::{Body,to_bytes},http::Request};use tower::ServiceExt;
    let (f,p,app,id)=start();p.mode.store(3,Ordering::SeqCst);app.reconcile_send_local(&f.account,&id).unwrap();
    let allowed=app.issue_token_local("owned record reader",vec![f.account.clone()],vec!["operations:read".into()],1).unwrap();
    let denied=app.issue_token_local("owned record denied",vec![f.account.clone()],vec!["accounts:read".into()],1).unwrap();
    app.enable_api(true);let router=werc_http_api::router(Arc::new(app));let queries=p.queries.load(Ordering::SeqCst);
    for (secret,expected) in [(&denied.secret,403),(&allowed.secret,200)] {
        let response=router.clone().oneshot(Request::builder().uri(format!("/api/v1/outbox/{id}/local-record?account_id={}",f.account)).header("Authorization",format!("Bearer {secret}")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),expected);let bytes=to_bytes(response.into_body(),65536).await.unwrap();
        if expected==200 {let value:serde_json::Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(value["data"]["state"],"written");assert!(!String::from_utf8_lossy(&bytes).contains("精确回执"));}
    }
    assert_eq!(p.queries.load(Ordering::SeqCst),queries);assert_eq!(p.submissions.load(Ordering::SeqCst),1);
}

#[test]
fn exact_media_receipt_retains_type_and_reconciles_after_reopen_without_resending() {
    let (f, p, app, _) = start();
    p.message_type.store((6u64 << 32) | 49, Ordering::SeqCst);
    let asset = app
        .upload_file_local(
            &f.account,
            "自动化测试.txt",
            b"owned exact media test".to_vec(),
        )
        .unwrap();
    let draft = app
        .create_file_send_local(&f.account, "filehelper", &asset.id, "owned-exact-file-once")
        .unwrap();
    app.approve_text_send_local(
        &f.account,
        &draft.metadata.id,
        &draft.metadata.request_digest,
        true,
    )
    .unwrap();
    app.execute_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(
        app.reconcile_send_local(&f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "LIVE_READ_FAILED"
    );
    p.mode.store(1, Ordering::SeqCst);
    p.inner.context.lock().unwrap().epoch = "owned-media-reconnect".into();
    let reopened = Application::new(
        Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap()),
        p.clone(),
    );
    p.message_type.store(3, Ordering::SeqCst);
    assert_eq!(
        reopened
            .reconcile_send_local(&f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "RECEIPT_MISMATCH"
    );
    p.message_type.store((6u64 << 32) | 49, Ordering::SeqCst);
    assert_eq!(
        reopened
            .reconcile_send_local(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::ServerAccepted
    );
    assert!(
        reopened
            .execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert_eq!(p.submissions.load(Ordering::SeqCst), 2);
}
