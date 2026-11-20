#![cfg(windows)]
#[path="support/owned_directory.rs"]
mod owned_directory;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{
    AppError, AppResult, Principal, capability::Capability, connection::*, outbox::*, receive::*,
};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

struct Receiver {
    repository: Arc<SqliteRepository>,
    account: String,
    context: Mutex<SendContext>,
    pending: Mutex<Option<ReceivedText>>,
    route: Mutex<Option<ReceivedRoute>>,
    voice: Mutex<Option<zeroize::Zeroizing<Vec<u8>>>>,
    dropped: AtomicUsize,
    fail_ack: AtomicBool,
    acks: AtomicUsize,
    downloads: AtomicUsize,
    download_failure: AtomicUsize,
    revoke_on_decrypt: Mutex<Option<String>>,
    revoke_on_download: Mutex<Option<String>>,
    revoke_on_decode: Mutex<Option<String>>,
    download_pause: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
}
impl CapabilityProvider for Receiver {
    fn download_received_video(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedVideoDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.download_received_image(context, &descriptor.media)?;
        Ok(zeroize::Zeroizing::new(
            if self.download_failure.load(Ordering::SeqCst) == 1 {
                vec![1]
            } else {
                include_bytes!("../../../tests/fixtures/owned-video.mp4").to_vec()
            },
        ))
    }
    fn download_received_file(
        &self,
        context: &SendContext,
        descriptor: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.download_received_image(context, descriptor)?;
        Ok(zeroize::Zeroizing::new(
            if self.download_failure.load(Ordering::SeqCst) == 1 {
                vec![1]
            } else {
                b"owned automatic test attachment".to_vec()
            },
        ))
    }

    fn decode_received_voice(&self, _: &[u8]) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        if let Some(id) = self.revoke_on_decode.lock().unwrap().take() {
            self.repository.revoke_token(&id)?;
        }
        // HTTP authorization fixture only. Source codec has its own real roundtrip test.
        Ok(zeroize::Zeroizing::new(owned_http_wav()))
    }

    fn download_received_image(
        &self,
        _: &SendContext,
        _: &werc_domain::received_media::ReceivedImageDescriptor,
    ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        if let Some(id) = self.revoke_on_download.lock().unwrap().take() {
            self.repository.revoke_token(&id)?;
        }
        if let Some((entered, resume)) = self.download_pause.lock().unwrap().take() {
            entered.send(()).unwrap();
            resume
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|_| AppError::new("OWNED_TIMEOUT", "fixture"))?;
        }
        match self.download_failure.load(Ordering::SeqCst) {
            1 => return Ok(zeroize::Zeroizing::new(vec![1])),
            2 => self.context.lock().unwrap().epoch = "changed-during-download".into(),
            _ => {}
        }
        Ok(zeroize::Zeroizing::new(
            include_bytes!("../../../tests/fixtures/owned-image.png").to_vec(),
        ))
    }
    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        if (c.starts_with("WeRC/file/") || c.starts_with("WeRC/video/"))
            && let Some(id) = self.revoke_on_decrypt.lock().unwrap().take()
        {
            self.repository.revoke_token(&id)?;
        }
        WindowsWechatProvider.unprotect_key(c, k)
    }
    fn resolve_send_context(&self, a: &str, t: &str) -> AppResult<SendContext> {
        assert_eq!(a, self.account);
        assert_eq!(t, "filehelper");
        Ok(self.context.lock().unwrap().clone())
    }
    fn poll_receive(&self, _: &str) -> AppResult<ReceiveBatch> {
        Ok(ReceiveBatch {
            route: self.route.lock().unwrap().clone(),
            inline_voice: self.voice.lock().unwrap().clone(),
            context: self.context.lock().unwrap().clone(),
            message: self.pending.lock().unwrap().clone(),
            dropped: self.dropped.load(Ordering::SeqCst) as u32,
        })
    }
    fn acknowledge_receive(&self, a: &str, e: &str, s: u64) -> AppResult<()> {
        assert_eq!(e, self.context.lock().unwrap().epoch);
        // Assert transaction already durable at the point an ACK can happen.
        assert_eq!(self.repository.receive_cursor(a)?.unwrap().sequence, s);
        assert!(!self.repository.received_texts(a, 0, 100)?.is_empty());
        self.acks.fetch_add(1, Ordering::SeqCst);
        if self.fail_ack.load(Ordering::SeqCst) {
            return Err(AppError::new("OWNED_ACK_LOST", "fixture only"));
        }
        self.pending.lock().unwrap().take();
        self.voice.lock().unwrap().take();
        Ok(())
    }
}
struct Fixture {
    path: std::path::PathBuf,
    account: String,
    repo: Arc<SqliteRepository>,
    provider: Arc<Receiver>,
    app: Application,
    _cleanup: owned_directory::OwnedDirectory,
}
#[tokio::test]
async fn bot_stream_forwards_both_conversations_resumes_and_closes_on_revocation() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let login = f.provider.context.lock().unwrap().sender_id.clone();
    for (sequence, sender) in [(1, "owned-person-a"), (2, "owned-person-b")] {
        *f.provider.route.lock().unwrap() = Some(ReceivedRoute {
            sender: sender.into(),
            receiver: login.clone(),
            conversation: sender.into(),
        });
        f.message(sequence, &format!("{}", 100 + sequence));
        f.step().unwrap();
    }
    let positions: Vec<_> = f.rows().iter().map(|r| r.position).collect();
    let reader = f
        .app
        .issue_token_local(
            "owned-stream",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let account = f.account.clone();
    let app = Arc::new(f.app);
    let router = werc_http_api::router(app.clone());
    let request = |after: Option<i64>| {
        let mut builder = Request::builder()
            .uri(format!(
                "/api/v1/messages/native/stream?account_id={account}&after=0"
            ))
            .header("Authorization", format!("Bearer {}", reader.secret));
        if let Some(after) = after {
            builder = builder.header("Last-Event-ID", after.to_string());
        }
        builder.body(Body::empty()).unwrap()
    };
    async fn frame(body: &mut Body) -> String {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), body.frame())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap()
    }
    let response = router.clone().oneshot(request(None)).await.unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    assert!(frame(&mut body).await.contains("receive.status"));
    assert!(frame(&mut body).await.contains("owned-person-a"));
    assert!(frame(&mut body).await.contains("owned-person-b"));
    drop(body);
    let mut resumed = router
        .oneshot(request(Some(positions[0])))
        .await
        .unwrap()
        .into_body();
    assert!(frame(&mut resumed).await.contains("receive.status"));
    let second = frame(&mut resumed).await;
    assert!(second.contains(&format!("id: {}", positions[1])));
    assert!(second.contains("owned-person-b"));
    assert!(!second.contains("owned-person-a"));
    app.revoke_token_local(&reader.info.id).unwrap();
    assert!(frame(&mut resumed).await.contains("stream.closed"));
}

fn video_observation(f: &Fixture) -> i64 {
    *f.provider.pending.lock().unwrap() = Some(ReceivedText {
        sequence: 1,
        native_message_id: "99943".into(),
        text: String::new(),
        message_type: 43,
        metadata: Some(format!(
            "<msg><videomsg aeskey=\"{}\" cdnvideourl=\"aabb\" length=\"100\"/></msg>",
            "ab".repeat(16)
        )),
    });
    f.step().unwrap();
    f.rows()[0].position
}
#[test]
fn video_cache_is_encrypted_reused_reopened_and_explicitly_removed() {
    let f = Fixture::new();
    let position = video_observation(&f);
    let cursor = f.app.receive_cursor_local(&f.account).unwrap();
    let asset = f.app.receive_video_local(&f.account, position).unwrap();
    assert_eq!(asset.video.width, 32);
    assert_eq!(
        f.app.receive_video_local(&f.account, position).unwrap().id,
        asset.id
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
    let encrypted: Vec<u8> = f
        .db()
        .query_row("SELECT ciphertext FROM received_video_assets", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(encrypted.len(), asset.size + 16);
    assert_ne!(
        &encrypted[..asset.size],
        include_bytes!("../../../tests/fixtures/owned-video.mp4")
    );
    assert_eq!(f.app.receive_cursor_local(&f.account).unwrap(), cursor);
    assert!(f.app.sends_local(&f.account).unwrap().is_empty());
    let offline = Application::new(
        Arc::new(SqliteRepository::open(&f.path).unwrap()),
        Arc::new(WindowsWechatProvider),
    );
    assert_eq!(
        offline
            .receive_video_local(&f.account, position)
            .unwrap()
            .id,
        asset.id
    );
    assert_eq!(
        &*offline
            .received_video_content_local(&f.account, position)
            .unwrap(),
        include_bytes!("../../../tests/fixtures/owned-video.mp4")
    );
    offline
        .delete_received_video_local(&f.account, position)
        .unwrap();
    assert!(
        offline
            .received_video_asset_local(&f.account, position)
            .unwrap()
            .is_none()
    );
    assert!(
        offline
            .received_video_content_local(&f.account, position)
            .is_err()
    );
}
#[test]
fn video_rejects_bad_content_changed_identity_and_wrong_type_without_saving() {
    for mode in [1, 2] {
        let f = Fixture::new();
        let pos = video_observation(&f);
        f.provider.download_failure.store(mode, Ordering::SeqCst);
        assert_eq!(
            f.app.receive_video_local(&f.account, pos).unwrap_err().code,
            if mode == 1 {
                "UNSUPPORTED_VIDEO"
            } else {
                "RECEIVE_CONTEXT_CHANGED"
            }
        );
        assert!(
            f.app
                .received_video_asset_local(&f.account, pos)
                .unwrap()
                .is_none()
        );
    }
    let f = Fixture::new();
    f.message(1, "99944");
    f.step().unwrap();
    assert!(
        f.app
            .receive_video_local(&f.account, f.rows()[0].position)
            .is_err()
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 0);
}
#[test]
fn video_ciphertext_and_full_metadata_are_authenticated() {
    for sql in [
        "UPDATE received_video_assets SET proof=printf('%064d',0)",
        "UPDATE received_video_assets SET ciphertext=zeroblob(length(ciphertext))",
        "UPDATE received_video_assets SET metadata=json_set(metadata,'$.video.width',33)",
        "UPDATE received_video_assets SET proof=printf('%065536d',0)",
    ] {
        let f = Fixture::new();
        let pos = video_observation(&f);
        f.app.receive_video_local(&f.account, pos).unwrap();
        f.db().execute(sql, []).unwrap();
        assert!(f.app.received_video_content_local(&f.account, pos).is_err());
        assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn video_transaction_rejects_stale_revisions_and_duplicate_without_orphan() {
    let f = Fixture::new();
    let pos = video_observation(&f);
    f.app.receive_video_local(&f.account, pos).unwrap();
    let id = f.rows()[0].payload.id.clone();
    let mut data = f.repo.received_video(&f.account, &id).unwrap();
    data.asset.id = uuid::Uuid::new_v4().to_string();
    let revision = f
        .repo
        .current_connection()
        .unwrap()
        .revision
        .parse::<i64>()
        .unwrap();
    let keys = f.repo.key_revision(&f.account).unwrap();
    for (r, k, code) in [
        (revision + 1, keys, "CONNECTION_CHANGED"),
        (revision, keys + 1, "CONNECTION_CHANGED"),
        (revision, keys, "RECEIVED_MEDIA_CONFLICT"),
    ] {
        assert_eq!(
            f.repo
                .save_received_video(
                    &data,
                    &werc_domain::video::VideoBinding {
                        observation_id: &id,
                        connection_revision: r,
                        key_revision: k,
                        proof: &"1".repeat(64)
                    }
                )
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(
            f.db()
                .query_row("SELECT count(*) FROM received_video_assets", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
impl Fixture {
    fn revoke(&self, sequence: u64, id: &str, original: &str, session: &str) {
        *self.provider.pending.lock().unwrap() = Some(ReceivedText {
            sequence, native_message_id: id.into(), text: String::new(), message_type: 10002,
            metadata: Some(format!("<sysmsg><revokemsg><session>{session}</session><newmsgid>{original}</newmsgid><replacemsg>自动化测试撤回通知</replacemsg></revokemsg></sysmsg>")),
        });
        self.step().unwrap();
    }
    fn new() -> Self {
        let cleanup =
            owned_directory::OwnedDirectory::new("werc-receive-fixture");
        let dir=cleanup.0.clone();
        let path = dir.join("state.sqlite3");
        let repo = Arc::new(SqliteRepository::open(&path).unwrap());
        let account = repo.register_account("owned receive fixture").unwrap().id;
        let sha = "1".repeat(64);
        repo.save_binding(&ClientBinding {
            account_id: account.clone(),
            installation: ClientInstallation {
                id: "owned".into(),
                executable_path: "not-executed".into(),
                version: "fixture".into(),
                architecture: "x86_64".into(),
                product_name: "fixture".into(),
                sha256: sha.clone(),
                process_ids: vec![],
                sources: vec![],
                compatibility: "fixture".into(),
                reason: "fixture".into(),
            },
            bound_at: "fixture".into(),
            checked_at: "fixture".into(),
            validation: "fixture".into(),
        })
        .unwrap();
        let provider = Arc::new(Receiver {
            repository: repo.clone(),
            account: account.clone(),
            context: Mutex::new(SendContext {
                sender_id: "owned-private-identity".into(),
                target_id: "filehelper".into(),
                target_display_name: "文件传输助手".into(),
                identity_verified: true,
                target_verified: true,
                epoch: "epoch-one".into(),
                installation_sha256: sha,
                adapter_sha256: "2".repeat(64),
                execution_mode: SendExecutionMode::Synthetic,
            }),
            pending: Mutex::new(None),
            route: Mutex::new(None),
            voice: Mutex::new(None),
            dropped: AtomicUsize::new(0),
            fail_ack: AtomicBool::new(false),
            acks: AtomicUsize::new(0),
            downloads: AtomicUsize::new(0),
            download_failure: AtomicUsize::new(0),
            revoke_on_decrypt: Mutex::new(None),
            revoke_on_download: Mutex::new(None),
            revoke_on_decode: Mutex::new(None),
            download_pause: Mutex::new(None),
        });
        let app = Application::new(repo.clone(), provider.clone());
        Self {
            path,
            account,
            repo,
            provider,
            app,
            _cleanup:cleanup,
        }
    }
    fn message(&self, sequence: u64, id: &str) {
        *self.provider.pending.lock().unwrap() = Some(ReceivedText {
            message_type: 1,
            metadata: None,
            sequence,
            native_message_id: id.into(),
            text: "owned-private-body-never-sent".into(),
        });
    }
    fn step(&self) -> AppResult<ReceiveProgress> {
        self.app.receive_once_local(&self.account)
    }
    fn rows(&self) -> Vec<ReceivedTextRow> {
        self.app
            .received_texts_local(&self.account, 0, 100)
            .unwrap()
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).unwrap()
    }
}

#[test]
fn preservation_exact_original_deduplicates_reopens_and_keeps_after_disable() {
    let f=Fixture::new();
    f.app.configure_message_preservation_local(&f.account,"filehelper",true).unwrap();
    f.message(1,"18446744073709551615"); f.step().unwrap();
    f.revoke(2,"200","18446744073709551615","filehelper");
    f.revoke(3,"201","18446744073709551615","filehelper");
    let status=f.app.step_message_preservation_local(&f.account).unwrap();
    assert_eq!(status.saved,1); assert_eq!(status.scanned_position,f.rows().last().unwrap().position);
    let page=f.app.preserved_messages_local(&f.account,None,None,None).unwrap();
    assert_eq!(page.data[0].state,"original_saved");
    assert_eq!(page.data[0].text.as_deref(),Some("owned-private-body-never-sent"));
    f.app.configure_message_preservation_local(&f.account,"filehelper",false).unwrap();
    let app=Application::new(Arc::new(SqliteRepository::open(&f.path).unwrap()),f.provider.clone());
    assert!(app.message_preservation_status_local(&f.account).unwrap().conversations.is_empty());
    assert_eq!(app.preserved_messages_local(&f.account,None,None,Some("original_saved")).unwrap().data.len(),1);
    for bytes in [std::fs::read(&f.path).unwrap(),std::fs::read(f.path.with_extension("sqlite3-wal")).unwrap_or_default()] {
        assert!(!bytes.windows(b"owned-private-body-never-sent".len()).any(|w|w==b"owned-private-body-never-sent"));
    }
}

#[test]
fn preservation_missing_original_fills_only_exact_identity_and_conversation() {
    let f=Fixture::new();
    f.app.configure_message_preservation_local(&f.account,"filehelper",true).unwrap();
    f.revoke(1,"201","123","filehelper");
    f.app.step_message_preservation_local(&f.account).unwrap();
    assert_eq!(f.app.preserved_messages_local(&f.account,None,None,None).unwrap().data[0].state,"original_not_observed");
    let login=f.provider.context.lock().unwrap().sender_id.clone();
    *f.provider.route.lock().unwrap()=Some(ReceivedRoute{sender:"owned-other".into(),receiver:login,conversation:"owned-other".into()});
    f.message(2,"123");f.step().unwrap();
    f.app.step_message_preservation_local(&f.account).unwrap();
    assert_eq!(f.app.preserved_messages_local(&f.account,None,None,None).unwrap().data[0].state,"original_not_observed");
    *f.provider.route.lock().unwrap()=None;
    f.message(3,"123");f.step().unwrap();
    f.app.step_message_preservation_local(&f.account).unwrap();
    assert_eq!(f.app.preserved_messages_local(&f.account,None,None,None).unwrap().data[0].state,"original_saved");
    assert!(f.app.preserved_messages_local(&f.account,None,Some("owned-other"),None).unwrap().data.is_empty());
    assert!(f.app.preserved_messages_local(&f.account,None,None,Some("unknown")).is_err());
}

#[test]
fn preservation_rejects_ambiguous_notices_and_rescan_does_not_multiply_errors() {
    let f=Fixture::new();f.app.configure_message_preservation_local(&f.account,"filehelper",true).unwrap();
    f.message(1,"123");f.step().unwrap();
    f.revoke(2,"201","123","owned-other");
    f.revoke(3,"202","0123","filehelper");
    f.revoke(4,"203","0","filehelper");
    let s=f.app.step_message_preservation_local(&f.account).unwrap();
    assert_eq!(s.saved,0);assert_eq!(s.unrecognized_notices,3);
    f.app.configure_message_preservation_local(&f.account,"owned-other",true).unwrap();
    let s=f.app.step_message_preservation_local(&f.account).unwrap();
    assert_eq!(s.saved,0);assert_eq!(s.unrecognized_notices,3);
    assert_eq!(f.rows().len(),4); // No suppression of the original receive stream.
}

#[test]
fn preservation_checkpoint_rolls_back_on_storage_failure() {
    let f=Fixture::new();f.app.configure_message_preservation_local(&f.account,"filehelper",true).unwrap();
    f.message(1,"123");f.step().unwrap();f.revoke(2,"201","123","filehelper");
    f.db().execute_batch("CREATE TRIGGER owned_preservation_failure BEFORE INSERT ON library_documents BEGIN SELECT RAISE(ABORT,'owned failure'); END;").unwrap();
    assert!(f.app.step_message_preservation_local(&f.account).is_err());
    assert_eq!(f.app.message_preservation_status_local(&f.account).unwrap().scanned_position,0);
    f.db().execute_batch("DROP TRIGGER owned_preservation_failure").unwrap();
    assert_eq!(f.app.step_message_preservation_local(&f.account).unwrap().saved,1);
}

#[test]
fn preservation_backup_restores_copies_without_enabling_another_account() {
    let f=Fixture::new();f.app.configure_message_preservation_local(&f.account,"filehelper",true).unwrap();
    f.message(1,"123");f.step().unwrap();f.revoke(2,"201","123","filehelper");
    f.app.step_message_preservation_local(&f.account).unwrap();
    let mut bytes=Vec::new();
    f.app.backup_library_local(&f.account,"owned-password-123","owned-preservation-backup",&mut bytes).unwrap();
    let restored=Fixture::new();let other=&restored.account;
    restored.app.restore_library_local(other,"owned-password-123","owned-preservation-restore",&mut std::io::Cursor::new(bytes)).unwrap();
    let status=restored.app.message_preservation_status_local(other).unwrap();
    assert!(status.conversations.is_empty());assert_eq!(status.saved,1);
    assert_eq!(restored.app.preserved_messages_local(other,None,None,None).unwrap().data[0].text.as_deref(),Some("owned-private-body-never-sent"));
}

#[tokio::test]
async fn preservation_api_and_mcp_enforce_read_and_write_scopes() {
    use axum::{body::Body,http::Request};use http_body_util::BodyExt;use tower::ServiceExt;
    let f=Fixture::new();
    let reader=f.app.issue_token_local("owned preservation read",vec![f.account.clone()],vec!["messages:read".into()],1).unwrap();
    let writer=f.app.issue_token_local("owned preservation write",vec![f.account.clone()],vec!["messages:read".into(),"operations:create".into()],1).unwrap();
    let denied=f.app.issue_token_local("owned preservation denied",vec![f.account.clone()],vec!["accounts:read".into()],1).unwrap();
    let other="owned-out-of-scope-account";
    f.app.enable_api(true);let account=f.account.clone();let app=Arc::new(f.app);let router=werc_http_api::router(app.clone());
    let call=|endpoint:&str,secret:&str,body:serde_json::Value| Request::builder().method("POST").uri(endpoint).header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json").body(Body::from(body.to_string())).unwrap();
    for (secret,action,expected) in [(&reader.secret,"status",200),(&reader.secret,"list",200),(&reader.secret,"configure",403),(&reader.secret,"step",403),(&writer.secret,"configure",200),(&writer.secret,"step",200),(&denied.secret,"status",403)] {
        let mut body=serde_json::json!({"action":action,"account_id":account});
        if action=="configure"{body["conversation"]="filehelper".into();body["enabled"]=true.into();}
        let response=router.clone().oneshot(call("/api/v1/messages/preservation",secret,body)).await.unwrap();
        let status=response.status();let bytes=response.into_body().collect().await.unwrap().to_bytes();assert_eq!(status,expected,"{}",String::from_utf8_lossy(&bytes));
    }
    assert_eq!(router.clone().oneshot(call("/api/v1/messages/preservation",&writer.secret,serde_json::json!({"action":"status","account_id":other}))).await.unwrap().status(),403);
    for (action,is_error) in [("status",false),("list",false),("configure",true),("step",true)] {
        let mut body=serde_json::json!({"action":action,"account_id":account});
        if action=="configure"{body["conversation"]="filehelper".into();body["enabled"]=false.into();}
        let rpc=serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"api_read","arguments":{"method":"POST","path":"/api/v1/messages/preservation","body":body}}});
        let response=router.clone().oneshot(call("/mcp/read",&writer.secret,rpc)).await.unwrap();
        let value:serde_json::Value=serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(value["result"]["isError"],is_error,"{value}");
    }
    assert_eq!(app.message_preservation_status_local(&account).unwrap().conversations,vec!["filehelper"]);
}

#[test]
fn durable_commit_precedes_ack_and_ack_loss_deduplicates_after_reopen() {
    let f = Fixture::new();
    f.message(1, "18446744073709551615");
    f.provider.fail_ack.store(true, Ordering::SeqCst);
    assert_eq!(f.step().err().unwrap().code, "OWNED_ACK_LOST");
    assert_eq!(f.rows().len(), 1);
    let reopened = Arc::new(SqliteRepository::open(&f.path).unwrap());
    let app = Application::new(reopened, f.provider.clone());
    f.provider.fail_ack.store(false, Ordering::SeqCst);
    let result = app.receive_once_local(&f.account).unwrap();
    assert!(!result.stored && result.acknowledged);
    assert_eq!(f.rows().len(), 1);
    assert_eq!(
        f.rows()[0].payload.message.native_message_id,
        "18446744073709551615"
    );
    let disk = std::fs::read(&f.path).unwrap();
    for secret in [
        "owned-private-body-never-sent",
        "owned-private-identity",
        "18446744073709551615",
    ] {
        assert!(!disk.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
}
#[test]
fn storage_failure_never_acknowledges_or_advances_checkpoint() {
    let f = Fixture::new();
    f.message(1, "5");
    f.db().execute_batch("CREATE TRIGGER owned_failure BEFORE INSERT ON received_texts BEGIN SELECT RAISE(ABORT,'owned failure'); END;").unwrap();
    assert!(f.step().is_err());
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 0);
    assert!(f.repo.receive_cursor(&f.account).unwrap().is_none());
    f.db().execute_batch("DROP TRIGGER owned_failure").unwrap();
    assert!(f.step().unwrap().stored);
}
#[test]
fn reconnect_and_overflow_gaps_are_durable_and_duplicate_native_ids_are_single_rows() {
    let f = Fixture::new();
    assert_eq!(f.step().unwrap().gaps, 1);
    f.message(1, "5");
    assert!(f.step().unwrap().stored);
    f.provider.dropped.store(3, Ordering::SeqCst);
    assert_eq!(f.step().unwrap().gaps, 2);
    assert_eq!(f.step().unwrap().gaps, 2);
    f.provider.context.lock().unwrap().epoch = "epoch-two".into();
    f.provider.dropped.store(0, Ordering::SeqCst);
    f.message(1, "5");
    let p = f.step().unwrap();
    assert_eq!(p.gaps, 3);
    assert!(!p.stored && p.acknowledged);
    assert_eq!(f.rows().len(), 1);
    f.message(3, "6");
    assert_eq!(f.step().unwrap().gaps, 4);
    assert_eq!(f.rows().len(), 2);
}
#[test]
fn ciphertext_tampering_and_cross_account_reads_are_rejected() {
    let f = Fixture::new();
    f.message(1, "7");
    f.step().unwrap();
    let principal = Principal {
        id: "owned".into(),
        account_ids: vec!["another-account".into()],
        scopes: vec!["messages:read".into()],
    };
    assert_eq!(
        f.app
            .received_texts(&principal, &f.account, 0, 100)
            .err()
            .unwrap()
            .code,
        "FORBIDDEN"
    );
    f.db()
        .execute(
            "UPDATE received_texts SET ciphertext=zeroblob(length(ciphertext))",
            [],
        )
        .unwrap();
    assert_eq!(
        f.app
            .received_texts_local(&f.account, 0, 100)
            .err()
            .unwrap()
            .code,
        "RECEIVE_CORRUPTED"
    );
}
#[test]
fn stale_connection_revision_and_cursor_fail_compare_and_swap() {
    let f = Fixture::new();
    f.step().unwrap();
    let c = f.repo.receive_cursor(&f.account).unwrap().unwrap();
    assert_eq!(
        f.repo
            .commit_receive(&ReceiveCommit {
                voice: None,
                account_id: &f.account,
                connection_revision: -1,
                expected: Some(&c),
                next: &c,
                message: None
            })
            .err()
            .unwrap()
            .code,
        "RECEIVE_CONTEXT_CHANGED"
    );
    let revision = f
        .repo
        .current_connection()
        .unwrap()
        .revision
        .parse()
        .unwrap();
    assert!(
        f.repo
            .commit_receive(&ReceiveCommit {
                voice: None,
                account_id: &f.account,
                connection_revision: revision,
                expected: None,
                next: &c,
                message: None
            })
            .is_err()
    );
    assert_eq!(f.repo.receive_cursor(&f.account).unwrap(), Some(c));
}
#[test]
fn sequence_regression_and_invalid_text_are_not_acknowledged() {
    let f = Fixture::new();
    f.message(3, "8");
    f.step().unwrap();
    f.message(2, "9");
    assert!(f.step().is_err());
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 1);
    f.message(4, "9");
    f.provider.pending.lock().unwrap().as_mut().unwrap().text = "x".repeat(16384);
    assert!(f.step().is_err());
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 1);
}

#[test]
fn native_identity_is_pinned_across_reconnect_and_missing_secret_is_not_regenerated() {
    let f = Fixture::new();
    f.provider.context.lock().unwrap().execution_mode = SendExecutionMode::Native;
    f.step().unwrap();
    let token = f.repo.native_identity(&f.account).unwrap().unwrap();
    assert_eq!(token.len(), 64);
    f.provider.context.lock().unwrap().sender_id = "different-owned-login".into();
    assert_eq!(f.step().err().unwrap().code, "NATIVE_ACCOUNT_MISMATCH");
    assert_eq!(f.repo.native_identity(&f.account).unwrap().unwrap(), token);
    f.db().execute("DELETE FROM send_secrets", []).unwrap();
    assert_eq!(f.step().err().unwrap().code, "OUTBOX_CORRUPTED");
    assert!(f.repo.send_secret(&f.account, None).unwrap().is_none());
}

#[test]
fn latest_page_and_forward_page_preserve_exact_order_without_duplicates() {
    let f = Fixture::new();
    for i in 1..=5 {
        f.message(i, &i.to_string());
        f.step().unwrap();
    }
    let latest = f.app.received_texts_local(&f.account, -1, 2).unwrap();
    assert_eq!(latest.len(), 2);
    assert_eq!(latest[0].payload.message.native_message_id, "4");
    assert_eq!(latest[1].payload.message.native_message_id, "5");
    let forward = f
        .app
        .received_texts_local(&f.account, latest[0].position, 2)
        .unwrap();
    assert_eq!(forward.len(), 1);
    assert_eq!(forward[0].payload.id, latest[1].payload.id);
    assert!(f.app.received_texts_local(&f.account, -2, 1).is_err());
    assert!(f.app.received_texts_local(&f.account, 0, 101).is_err());
}

#[tokio::test]
async fn native_read_api_enforces_scope_and_returns_only_persisted_observations() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    f.message(1, "10");
    f.step().unwrap();
    let reader = f
        .app
        .issue_token_local(
            "owned-reader",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    let denied = f
        .app
        .issue_token_local(
            "owned-denied",
            vec![f.account.clone()],
            vec!["accounts:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let account = f.account.clone();
    let app = Arc::new(f.app);
    let router = werc_http_api::router(app.clone());
    let request = |secret: &str, query: &str| {
        Request::builder()
            .uri(format!(
                "/api/v1/messages/native?account_id={account}{query}"
            ))
            .header("Authorization", format!("Bearer {secret}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request(&denied.secret, ""))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(&reader.secret, "&limit=101"))
            .await
            .unwrap()
            .status(),
        400
    );
    let result = router
        .clone()
        .oneshot(request(&reader.secret, ""))
        .await
        .unwrap();
    assert_eq!(result.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&result.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["historical_complete"], false);
    assert_eq!(body["data_mode"], "native_observations");
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    app.revoke_token_local(&reader.info.id).unwrap();
    assert_eq!(
        router
            .oneshot(request(&reader.secret, ""))
            .await
            .unwrap()
            .status(),
        401
    );
}

#[test]
fn conflicting_content_is_rejected_but_retained_history_does_not_stop_receiving() {
    let f = Fixture::new();
    f.message(1, "11");
    f.step().unwrap();
    f.message(2, "11");
    f.provider.pending.lock().unwrap().as_mut().unwrap().text = "different fixture content".into();
    assert_eq!(f.step().err().unwrap().code, "RECEIVE_CONTENT_CONFLICT");
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 1);
    f.message(2, "12");
    f.db()
        .execute(
            "UPDATE receive_usage SET messages=100000,bytes=268435456",
            [],
        )
        .unwrap();
    assert!(f.step().unwrap().acknowledged);
    assert_eq!(f.rows().len(), 2);
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 2);
}

#[test]
fn cursor_tampering_cannot_hide_a_gap_or_advance_consumption() {
    let f = Fixture::new();
    f.step().unwrap();
    f.db()
        .execute(
            "UPDATE receive_streams SET cursor=json_set(cursor,'$.gaps',0)",
            [],
        )
        .unwrap();
    f.message(1, "13");
    assert_eq!(f.step().err().unwrap().code, "RECEIVE_CORRUPTED");
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 0);
}

#[test]
fn v11_wal_upgrade_preserves_existing_protected_bytes_and_verified_backup() {
    let cleanup = owned_directory::OwnedDirectory::new("werc-receive-upgrade");
    let dir = cleanup.0.clone();
    let path = dir.join("state.sqlite3");
    let old = rusqlite::Connection::open(&path).unwrap();
    for sql in [
        include_str!("../../infrastructure/migrations/0001.sql"),
        include_str!("../../infrastructure/migrations/0002.sql"),
        include_str!("../../infrastructure/migrations/0003.sql"),
        include_str!("../../infrastructure/migrations/0004.sql"),
        include_str!("../../infrastructure/migrations/0005.sql"),
        include_str!("../../infrastructure/migrations/0006.sql"),
        include_str!("../../infrastructure/migrations/0007.sql"),
        include_str!("../../infrastructure/migrations/0008.sql"),
        include_str!("../../infrastructure/migrations/0009.sql"),
        include_str!("../../infrastructure/migrations/0010.sql"),
        include_str!("../../infrastructure/migrations/0011.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute_batch("PRAGMA journal_mode=WAL; INSERT INTO accounts VALUES('owned-legacy','owned','2026-11-11'); INSERT INTO send_secrets VALUES('owned-legacy',X'0102030405');").unwrap();
    let repository = SqliteRepository::open(&path).unwrap();
    assert_eq!(
        repository
            .send_secret("owned-legacy", None)
            .unwrap()
            .unwrap(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(
        old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        22
    );
    let backups = std::fs::read_dir(&dir)
        .unwrap()
        .map(|r| r.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains("pre-v12"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    let backup = rusqlite::Connection::open_with_flags(
        &backups[0],
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(
        backup
            .query_row("SELECT wrapped_key FROM send_secrets", [], |r| r
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        [1, 2, 3, 4, 5]
    );
    assert_eq!(
        backup
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[test]
fn typed_metadata_is_encrypted_durable_and_ack_loss_never_duplicates() {
    let f = Fixture::new();
    let metadata = "<msg><img aeskey=\"owned-secret-not-a-real-key\"/></msg>";
    *f.provider.pending.lock().unwrap() = Some(ReceivedText {
        sequence: 1,
        native_message_id: "19".into(),
        text: String::new(),
        message_type: 3,
        metadata: Some(metadata.into()),
    });
    f.provider.fail_ack.store(true, Ordering::SeqCst);
    assert_eq!(f.step().err().unwrap().code, "OWNED_ACK_LOST");
    let rows = f.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload.schema_version, 2);
    assert!(rows[0].payload.message.text.is_empty());
    assert_eq!(rows[0].payload.message.metadata.as_deref(), Some(metadata));
    let disk = std::fs::read(&f.path).unwrap();
    assert!(
        !disk
            .windows(metadata.len())
            .any(|b| b == metadata.as_bytes())
    );
    f.provider.fail_ack.store(false, Ordering::SeqCst);
    let reopened = Application::new(
        Arc::new(SqliteRepository::open(&f.path).unwrap()),
        f.provider.clone(),
    );
    let result = reopened.receive_once_local(&f.account).unwrap();
    assert!(!result.stored && result.acknowledged);
    assert_eq!(f.rows().len(), 1);
    *f.provider.pending.lock().unwrap() = Some(ReceivedText {
        sequence: 2,
        native_message_id: "19".into(),
        text: String::new(),
        message_type: 34,
        metadata: Some(metadata.into()),
    });
    assert!(f.step().is_err(), "同一ID改变类型不能静默替换或确认");
    assert_eq!(f.rows()[0].payload.message.message_type, 3);
}
#[test]
fn additional_observed_types_survive_reopen_and_project_safe_display_cards() {
    let f=Fixture::new();
    for (i,(kind,metadata,expected)) in [
        (37,"<msg content='自动化测试验证' ticket='private-ticket'/>","friend_request"),
        (42,"<msg username='owned-contact' nickname='自动化测试名片'/>","contact"),
        (48,"<msg><location x='31' y='121' label='自动化测试位置'/></msg>","location"),
        (50,"<voipmsg>自动化测试通话</voipmsg>","call"),
        (10002,"<sysmsg><revokemsg><session>filehelper</session><newmsgid>18446744073709551615</newmsgid><replacemsg>自动化测试撤回</replacemsg></revokemsg></sysmsg>","revoke"),
    ].into_iter().enumerate() {
        *f.provider.pending.lock().unwrap()=Some(ReceivedText{sequence:i as u64+1,native_message_id:(100+i).to_string(),text:String::new(),message_type:kind,metadata:Some(metadata.into())});
        assert!(f.step().unwrap().stored);
        let reopened=Application::new(Arc::new(SqliteRepository::open(&f.path).unwrap()),f.provider.clone());
        let rows=reopened.received_texts_local(&f.account,0,100).unwrap();
        let row=rows.iter().find(|r|r.payload.message.message_type==kind).unwrap();
        assert_eq!(row.payload.message.metadata.as_deref(),Some(metadata));
        let card=serde_json::to_value(&row.card).unwrap();
        assert_eq!(card["detail"]["kind"].as_str().or_else(||card["kind"].as_str()),Some(expected));
        assert!(!card.to_string().contains("private-ticket"));
    }
}
#[test]
fn long_utf8_text_and_media_types_preserve_content_but_invalid_envelopes_never_ack() {
    let f = Fixture::new();
    f.message(1, "30");
    let long = "自有文字".repeat(1000);
    f.provider.pending.lock().unwrap().as_mut().unwrap().text = long.clone();
    assert!(f.step().unwrap().stored);
    assert_eq!(f.rows()[0].payload.message.text, long);
    for (i, kind) in [34, 43, 47, 49, 62, 10000, 10002].into_iter().enumerate() {
        *f.provider.pending.lock().unwrap() = Some(ReceivedText {
            sequence: 2 + i as u64,
            native_message_id: (31 + i).to_string(),
            text: String::new(),
            message_type: kind,
            metadata: Some("<owned/>".into()),
        });
        assert!(f.step().unwrap().stored);
    }
    let acks = f.provider.acks.load(Ordering::SeqCst);
    for (kind, text, metadata) in [
        (999, "", Some("<owned/>")),
        (3, "fake text", Some("<owned/>")),
        (1, "text", Some("<owned/>")),
        (34, "", None),
        (34, "", Some("bad\0value")),
    ] {
        *f.provider.pending.lock().unwrap() = Some(ReceivedText {
            sequence: 10,
            native_message_id: "99".into(),
            text: text.into(),
            message_type: kind,
            metadata: metadata.map(str::to_owned),
        });
        assert_eq!(f.step().err().unwrap().code, "RECEIVE_PROTOCOL_INVALID");
    }
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), acks);
    assert_eq!(f.rows().len(), 8);
}
#[test]
fn legacy_text_json_stays_identical_and_decodes_as_text() {
    let value = serde_json::json!({"sequence":1,"native_message_id":"2","text":"owned"});
    let old: ReceivedText = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(old.message_type, 1);
    assert!(old.metadata.is_none());
    assert_eq!(serde_json::to_value(old).unwrap(), value);
}

fn image_observation(f: &Fixture) -> i64 {
    *f.provider.pending.lock().unwrap() = Some(ReceivedText {
        sequence: 1,
        native_message_id: "9991".into(),
        text: String::new(),
        message_type: 3,
        metadata: Some(format!(
            "<msg><img aeskey=\"{}\" cdnmidimgurl=\"aabbccdd\" length=\"20\"/></msg>",
            "ab".repeat(16)
        )),
    });
    f.step().unwrap();
    f.rows()[0].position
}
#[test]
fn received_image_download_is_encrypted_reopens_and_never_creates_send() {
    let f = Fixture::new();
    let pos = image_observation(&f);
    let before = f.app.receive_cursor_local(&f.account).unwrap();
    let asset = f.app.receive_image_file_local(&f.account, pos).unwrap();
    assert_eq!(
        f.app
            .download_file(
                &Principal {
                    id: "fixture".into(),
                    account_ids: vec![f.account.clone()],
                    scopes: vec!["media:read".into()]
                },
                &f.account,
                &asset.id
            )
            .unwrap()
            .bytes
            .as_slice(),
        &werc_domain::image::canonical_png(include_bytes!(
            "../../../tests/fixtures/owned-image.png"
        ))
        .unwrap()
        .1[..]
    );
    assert!(f.app.sends_local(&f.account).unwrap().is_empty());
    assert_eq!(f.app.receive_cursor_local(&f.account).unwrap(), before);
    let disk = std::fs::read(&f.path).unwrap();
    let plain = werc_domain::image::canonical_png(include_bytes!(
        "../../../tests/fixtures/owned-image.png"
    ))
    .unwrap()
    .1;
    assert!(!disk.windows(plain.len()).any(|w| w == plain.as_slice()));
    let reopened = Application::new(
        Arc::new(SqliteRepository::open(&f.path).unwrap()),
        Arc::new(WindowsWechatProvider),
    );
    assert_eq!(
        reopened
            .file_preview_local(&f.account, &asset.id)
            .unwrap()
            .asset
            .sha256,
        asset.sha256
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
}
#[test]
fn received_image_download_rejects_missing_wrong_type_or_identity_before_native() {
    let f = Fixture::new();
    f.message(1, "9992");
    f.step().unwrap();
    let pos = f.rows()[0].position;
    assert!(f.app.receive_image_file_local(&f.account, pos).is_err());
    assert!(
        f.app
            .receive_image_file_local(&f.account, pos + 100)
            .is_err()
    );
    assert!(f.app.receive_image_file_local(&f.account, 0).is_err());
    let f = Fixture::new();
    let pos = image_observation(&f);
    f.provider.context.lock().unwrap().sender_id = "other-owned-identity".into();
    assert_eq!(
        f.app
            .receive_image_file_local(&f.account, pos)
            .err()
            .unwrap()
            .code,
        "RECEIVE_CONTEXT_CHANGED"
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 0);
}
#[test]
fn received_image_download_rejects_changed_size_or_connection_without_saving() {
    for mode in [1, 2] {
        let f = Fixture::new();
        let pos = image_observation(&f);
        f.provider.download_failure.store(mode, Ordering::SeqCst);
        assert_eq!(
            f.app
                .receive_image_file_local(&f.account, pos)
                .err()
                .unwrap()
                .code,
            if mode == 1 {
                "UNSUPPORTED_RECEIVED_IMAGE"
            } else {
                "RECEIVE_CONTEXT_CHANGED"
            }
        );
        assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
        assert!(f.app.sends_local(&f.account).unwrap().is_empty());
    }
}

#[test]
fn received_image_cache_survives_offline_reopen_and_deletion_removes_binding() {
    let f = Fixture::new();
    let pos = image_observation(&f);
    let first = f.app.receive_image_file_local(&f.account, pos).unwrap();
    assert_eq!(
        f.app.receive_image_file_local(&f.account, pos).unwrap().id,
        first.id
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
    let offline = Application::new(
        Arc::new(SqliteRepository::open(&f.path).unwrap()),
        Arc::new(WindowsWechatProvider),
    );
    assert_eq!(
        offline
            .receive_image_file_local(&f.account, pos)
            .unwrap()
            .id,
        first.id
    );
    assert!(
        !offline
            .received_image_preview_local(&f.account, pos)
            .unwrap()
            .is_empty()
    );
    f.repo.delete_file(&f.account, &first.id).unwrap();
    assert!(
        offline
            .received_image_file_asset_local(&f.account, pos)
            .unwrap()
            .is_none()
    );
    assert!(
        offline
            .received_image_preview_local(&f.account, pos)
            .is_err()
    );
    assert_ne!(
        f.app.receive_image_file_local(&f.account, pos).unwrap().id,
        first.id
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 2);
}
#[test]
fn received_image_tampered_link_rejects_without_download_or_wrong_preview() {
    let f = Fixture::new();
    let pos = image_observation(&f);
    f.app.receive_image_file_local(&f.account, pos).unwrap();
    f.db()
        .execute(
            "UPDATE received_image_assets SET proof=?1",
            ["0".repeat(64)],
        )
        .unwrap();
    assert_eq!(
        f.app
            .received_image_preview_local(&f.account, pos)
            .err()
            .unwrap()
            .code,
        "RECEIVED_MEDIA_CORRUPTED"
    );
    assert!(f.app.receive_image_file_local(&f.account, pos).is_err());
    f.db()
        .execute(
            "UPDATE received_image_assets SET proof=?1",
            ["0".repeat(65536)],
        )
        .unwrap();
    assert!(
        f.app
            .received_image_file_asset_local(&f.account, pos)
            .is_err()
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
}
#[test]
fn received_image_transaction_rejects_stale_revision_and_duplicate_without_orphan() {
    use werc_domain::received_media::ReceivedImageBinding;
    let f = Fixture::new();
    let pos = image_observation(&f);
    let asset = f.app.receive_image_file_local(&f.account, pos).unwrap();
    let mut protected = f.repo.file(&f.account, &asset.id).unwrap();
    protected.asset.id = uuid::Uuid::new_v4().to_string();
    let observation = f.rows()[0].payload.id.clone();
    let revision = f
        .repo
        .current_connection()
        .unwrap()
        .revision
        .parse::<i64>()
        .unwrap();
    let proof = "a".repeat(64);
    for (r, code) in [
        (revision + 1, "CONNECTION_CHANGED"),
        (revision, "RECEIVED_MEDIA_CONFLICT"),
    ] {
        assert_eq!(
            f.repo
                .save_received_image_file(
                    &protected,
                    &ReceivedImageBinding {
                        observation_id: &observation,
                        connection_revision: r,
                        proof: &proof
                    }
                )
                .err()
                .unwrap()
                .code,
            code
        );
        assert_eq!(f.repo.files(&f.account).unwrap().len(), 1);
    }
}
#[test]
fn receive_commit_and_ack_continue_while_image_download_waits() {
    use std::{sync::mpsc, time::Duration};
    let f = Fixture::new();
    let pos = image_observation(&f);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    *f.provider.download_pause.lock().unwrap() = Some((entered_tx, resume_rx));
    std::thread::scope(|scope| {
        let download = scope.spawn(|| f.app.receive_image_file_local(&f.account, pos));
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            f.app
                .receive_image_file_local(&f.account, pos)
                .err()
                .unwrap()
                .code,
            "RECEIVED_MEDIA_BUSY"
        );
        f.message(2, "owned-during-download");
        // Native IDs are decimal strings.
        f.provider
            .pending
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .native_message_id = "9993".into();
        let (done_tx, done_rx) = mpsc::channel();
        let fixture = &f;
        scope.spawn(move || {
            done_tx.send(fixture.step()).unwrap();
        });
        let received = done_rx.recv_timeout(Duration::from_secs(2));
        resume_tx.send(()).unwrap();
        let result = received
            .expect("receiving must finish before releasing the download")
            .unwrap();
        assert!(result.stored && result.acknowledged);
        download.join().unwrap().unwrap();
    });
    assert_eq!(f.rows().len(), 2);
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 2);
    assert!(f.app.sends_local(&f.account).unwrap().is_empty());
}

#[tokio::test]
async fn native_image_api_requires_explicit_write_and_read_never_downloads() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let pos = image_observation(&f);
    let reader = f
        .app
        .issue_token_local(
            "reader",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:read".into()],
            1,
        )
        .unwrap();
    let writer = f
        .app
        .issue_token_local(
            "writer",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let media_only = f
        .app
        .issue_token_local(
            "media only",
            vec![f.account.clone()],
            vec!["media:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let app = Arc::new(f.app);
    let router = werc_http_api::router(app);
    let req = |method: &str, key: &str| {
        Request::builder()
            .method(method)
            .uri(format!(
                "/api/v1/messages/native/{pos}/image?account_id={}",
                f.account
            ))
            .header("Authorization", format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(req("GET", &reader.secret))
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &reader.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &media_only.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 0);
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &writer.secret))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &writer.secret))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("GET", &writer.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router.oneshot(req("GET", &reader.secret)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["content-type"], "image/png");
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .starts_with(b"\x89PNG")
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_image_api_rechecks_revocation_after_download_and_decryption() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let pos = image_observation(&f);
    let reader = f
        .app
        .issue_token_local(
            "reader",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:read".into()],
            1,
        )
        .unwrap();
    let writer = f
        .app
        .issue_token_local(
            "writer",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let app = Arc::new(f.app);
    let router = werc_http_api::router(app.clone());
    let req = |method: &str, key: &str| {
        Request::builder()
            .method(method)
            .uri(format!(
                "/api/v1/messages/native/{pos}/image?account_id={}",
                f.account
            ))
            .header("Authorization", format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap()
    };
    *f.provider.revoke_on_download.lock().unwrap() = Some(writer.info.id.clone());
    let response = router
        .clone()
        .oneshot(req("POST", &writer.secret))
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let json: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(json.get("data").is_none());
    assert!(
        app.received_image_file_asset_local(&f.account, pos)
            .unwrap()
            .is_some()
    );
    *f.provider.revoke_on_decrypt.lock().unwrap() = Some(reader.info.id.clone());
    let response = router.oneshot(req("GET", &reader.secret)).await.unwrap();
    assert_eq!(response.status(), 401);
    assert_ne!(response.headers()["content-type"], "image/png");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!body.windows(4).any(|w| w == b"\x89PNG"));
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
}

// These are deliberately arbitrary binary persistence fixtures, not SILK decoding claims.
fn voice_pending(f: &Fixture, bytes: Option<Vec<u8>>) {
    f.message(1, "987654");
    let mut pending = f.provider.pending.lock().unwrap();
    let m = pending.as_mut().unwrap();
    m.message_type = 34;
    m.text.clear();
    m.metadata = Some("<msg><voicemsg /></msg>".into());
    *f.provider.voice.lock().unwrap() = bytes.map(zeroize::Zeroizing::new);
}
#[test]
fn voice_commit_is_atomic_on_asset_link_failure_and_ack_loss_reuses_asset() {
    let f = Fixture::new();
    let bytes = vec![0, 1, 2, 0, 255, 4];
    voice_pending(&f, Some(bytes.clone()));
    f.db().execute_batch("CREATE TRIGGER owned_voice_failure BEFORE INSERT ON received_voice_assets BEGIN SELECT RAISE(ABORT,'owned'); END;").unwrap();
    assert!(f.step().is_err());
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 0);
    assert!(f.rows().is_empty());
    assert!(f.repo.files(&f.account).unwrap().is_empty());
    assert!(f.repo.receive_cursor(&f.account).unwrap().is_none());
    f.db()
        .execute_batch("DROP TRIGGER owned_voice_failure;")
        .unwrap();
    f.provider.fail_ack.store(true, Ordering::SeqCst);
    assert_eq!(f.step().err().unwrap().code, "OWNED_ACK_LOST");
    let pos = f.rows()[0].position;
    let asset = f
        .app
        .received_voice_file_asset_local(&f.account, pos)
        .unwrap()
        .unwrap();
    assert_eq!(
        &*f.app.received_voice_silk_local(&f.account, pos).unwrap(),
        &bytes
    );
    let reopened = Arc::new(SqliteRepository::open(&f.path).unwrap());
    let offline = Application::new(reopened, f.provider.clone());
    assert_eq!(
        &*offline.received_voice_silk_local(&f.account, pos).unwrap(),
        &bytes
    );
    f.provider.fail_ack.store(false, Ordering::SeqCst);
    let progress = offline.receive_once_local(&f.account).unwrap();
    assert!(!progress.stored && progress.acknowledged);
    assert_eq!(f.repo.files(&f.account).unwrap().len(), 1);
    assert_eq!(
        offline
            .received_voice_file_asset_local(&f.account, pos)
            .unwrap()
            .unwrap()
            .id,
        asset.id
    );
    assert!(f.app.sends_local(&f.account).unwrap().is_empty());
}
#[test]
fn voice_duplicate_content_conflict_does_not_ack_or_overwrite() {
    let f = Fixture::new();
    voice_pending(&f, Some(vec![1, 2, 0, 4]));
    f.provider.fail_ack.store(true, Ordering::SeqCst);
    assert!(f.step().is_err());
    *f.provider.voice.lock().unwrap() = Some(zeroize::Zeroizing::new(vec![1, 2, 0, 5]));
    assert_eq!(f.step().err().unwrap().code, "RECEIVE_CONTENT_CONFLICT");
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 1);
    assert_eq!(
        &*f.app
            .received_voice_silk_local(&f.account, f.rows()[0].position)
            .unwrap(),
        &[1, 2, 0, 4]
    );
    assert_eq!(f.repo.files(&f.account).unwrap().len(), 1);
}
#[test]
fn voice_absent_bytes_preserves_metadata_and_invalid_associations_fail() {
    let f = Fixture::new();
    voice_pending(&f, None);
    assert!(f.step().unwrap().acknowledged);
    assert!(
        f.app
            .received_voice_file_asset_local(&f.account, f.rows()[0].position)
            .unwrap()
            .is_none()
    );
    for bytes in [vec![], vec![1; 262145]] {
        let g = Fixture::new();
        voice_pending(&g, Some(bytes));
        assert!(g.step().is_err());
        assert_eq!(g.provider.acks.load(Ordering::SeqCst), 0);
        assert!(g.rows().is_empty());
    }
    let g = Fixture::new();
    g.message(1, "222");
    *g.provider.voice.lock().unwrap() = Some(zeroize::Zeroizing::new(vec![1]));
    assert!(g.step().is_err());
    assert!(g.rows().is_empty());
}
#[test]
fn voice_link_authentication_ciphertext_expiry_and_delete() {
    for action in ["proof", "ciphertext", "expiry", "delete"] {
        let f = Fixture::new();
        voice_pending(&f, Some(vec![0, 1, 2, 3]));
        f.step().unwrap();
        let pos = f.rows()[0].position;
        let asset = f
            .app
            .received_voice_file_asset_local(&f.account, pos)
            .unwrap()
            .unwrap();
        match action {
            "proof" => {
                f.db()
                    .execute(
                        "UPDATE received_voice_assets SET proof=?1",
                        ["0".repeat(64)],
                    )
                    .unwrap();
            }
            "ciphertext" => {
                f.db()
                    .execute(
                        "UPDATE file_assets SET ciphertext=zeroblob(length(ciphertext))",
                        [],
                    )
                    .unwrap();
            }
            "expiry" => {
                f.db()
                    .execute(
                        "UPDATE file_assets SET expires_at='2000-01-01T00:00:00Z'",
                        [],
                    )
                    .unwrap();
            }
            _ => {
                f.app.delete_file_local(&f.account, &asset.id).unwrap();
            }
        }
        assert!(f.app.received_voice_silk_local(&f.account, pos).is_err());
        if action == "delete" {
            assert_eq!(
                f.db()
                    .query_row("SELECT count(*) FROM received_voice_assets", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }
}

fn owned_http_wav() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&38u32.to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&16000u32.to_le_bytes());
    b.extend_from_slice(&32000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&1000i16.to_le_bytes());
    b
}
#[tokio::test]
async fn native_voice_api_requires_both_scopes_and_rechecks_after_decrypt_and_decode() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    for revoke in ["none", "decrypt", "decode"] {
        let f = Fixture::new();
        voice_pending(&f, Some(vec![1, 0, 2, 3]));
        f.step().unwrap();
        let pos = f.rows()[0].position;
        let full = f
            .app
            .issue_token_local(
                "full",
                vec![f.account.clone()],
                vec!["messages:read".into(), "media:read".into()],
                1,
            )
            .unwrap();
        let no_media = f
            .app
            .issue_token_local(
                "no media",
                vec![f.account.clone()],
                vec!["messages:read".into()],
                1,
            )
            .unwrap();
        let no_messages = f
            .app
            .issue_token_local(
                "no messages",
                vec![f.account.clone()],
                vec!["media:read".into()],
                1,
            )
            .unwrap();
        f.app.enable_api(true);
        let router = werc_http_api::router(Arc::new(f.app));
        let req = |secret: &str| {
            Request::builder()
                .uri(format!(
                    "/api/v1/messages/native/{pos}/voice?account_id={}",
                    f.account
                ))
                .header("Authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap()
        };
        for token in [&no_media, &no_messages] {
            assert_eq!(
                router
                    .clone()
                    .oneshot(req(&token.secret))
                    .await
                    .unwrap()
                    .status(),
                403
            );
        }
        let other_request = Request::builder()
            .uri(format!(
                "/api/v1/messages/native/{pos}/voice?account_id=owned-other"
            ))
            .header("Authorization", format!("Bearer {}", full.secret))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router
                .clone()
                .oneshot(other_request)
                .await
                .unwrap()
                .status(),
            403
        );
        if revoke == "decrypt" {
            *f.provider.revoke_on_decrypt.lock().unwrap() = Some(full.info.id.clone());
        }
        if revoke == "decode" {
            *f.provider.revoke_on_decode.lock().unwrap() = Some(full.info.id.clone());
        }
        let response = router.oneshot(req(&full.secret)).await.unwrap();
        if revoke == "none" {
            assert_eq!(response.status(), 200);
            assert_eq!(response.headers()["content-type"], "audio/wav");
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .as_ref(),
                owned_http_wav()
            );
        } else {
            assert_eq!(response.status(), 401);
            let body = response.into_body().collect().await.unwrap().to_bytes();
            assert!(!body.windows(4).any(|w| w == b"RIFF"));
        }
        assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 0);
        assert!(f.repo.sends(&f.account).unwrap().is_empty());
    }
}

fn file_observation(f: &Fixture) -> i64 {
    *f.provider.pending.lock().unwrap() = Some(ReceivedText {
        sequence: 1,
        native_message_id: "9991".into(),
        text: String::new(),
        message_type: 49,
        metadata: Some(format!(
            "<msg><appmsg><title><![CDATA[自动化测试.txt]]></title><type>6</type><appattach><totallen>31</totallen><cdnattachurl>aabbccdd</cdnattachurl><aeskey>{}</aeskey></appattach></appmsg></msg>",
            "ab".repeat(16)
        )),
    });
    f.step().unwrap();
    f.rows()[0].position
}
#[test]
fn received_file_cache_is_encrypted_and_survives_offline_reopen_without_sending() {
    let f = Fixture::new();
    let pos = file_observation(&f);
    let before = f.app.receive_cursor_local(&f.account).unwrap();
    let asset = f.app.receive_file_local(&f.account, pos).unwrap();
    assert_eq!(
        f.app.receive_file_local(&f.account, pos).unwrap().id,
        asset.id
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
    assert!(f.app.sends_local(&f.account).unwrap().is_empty());
    assert_eq!(f.app.receive_cursor_local(&f.account).unwrap(), before);
    let disk = std::fs::read(&f.path).unwrap();
    assert!(
        !disk
            .windows(30)
            .any(|w| w == b"owned automatic test attachment")
    );
    let offline = Application::new(
        Arc::new(SqliteRepository::open(&f.path).unwrap()),
        Arc::new(WindowsWechatProvider),
    );
    let data = offline
        .received_file_content_local(&f.account, pos)
        .unwrap();
    assert_eq!(data.file_name, "自动化测试.txt");
    assert_eq!(&*data.bytes, b"owned automatic test attachment");
    f.repo.delete_file(&f.account, &asset.id).unwrap();
    assert!(
        offline
            .received_file_asset_local(&f.account, pos)
            .unwrap()
            .is_none()
    );
}
#[test]
fn received_file_invalid_size_or_changed_identity_is_not_saved() {
    for mode in [1, 2] {
        let f = Fixture::new();
        let pos = file_observation(&f);
        f.provider.download_failure.store(mode, Ordering::SeqCst);
        assert!(f.app.receive_file_local(&f.account, pos).is_err());
        assert!(f.repo.files(&f.account).unwrap().is_empty());
    }
}
#[test]
fn received_file_link_tampering_is_rejected() {
    let f = Fixture::new();
    let pos = file_observation(&f);
    f.app.receive_file_local(&f.account, pos).unwrap();
    rusqlite::Connection::open(&f.path)
        .unwrap()
        .execute("UPDATE received_file_assets SET proof=?1", ["0".repeat(64)])
        .unwrap();
    assert!(f.app.received_file_content_local(&f.account, pos).is_err());
    assert!(f.app.receive_file_local(&f.account, pos).is_err());
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_file_api_requires_explicit_write_and_read_never_downloads() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let pos = file_observation(&f);
    let reader = f
        .app
        .issue_token_local(
            "reader",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:read".into()],
            1,
        )
        .unwrap();
    let writer = f
        .app
        .issue_token_local(
            "writer",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let media_only = f
        .app
        .issue_token_local(
            "media only",
            vec![f.account.clone()],
            vec!["media:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let app = Arc::new(f.app);
    let router = werc_http_api::router(app);
    let req = |method: &str, key: &str| {
        Request::builder()
            .method(method)
            .uri(format!(
                "/api/v1/messages/native/{pos}/file?account_id={}",
                f.account
            ))
            .header("Authorization", format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(req("GET", &reader.secret))
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &reader.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &media_only.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 0);
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &writer.secret))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &writer.secret))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("GET", &writer.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router.oneshot(req("GET", &reader.secret)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .starts_with(b"owned automatic test attachment")
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_video_api_explicit_download_scopes_revocation_and_removal() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let pos = video_observation(&f);
    let issue = |name, scopes: Vec<&str>| {
        f.app
            .issue_token_local(
                name,
                vec![f.account.clone()],
                scopes.into_iter().map(str::to_owned).collect(),
                1,
            )
            .unwrap()
    };
    let reader = issue("reader", vec!["messages:read", "media:read"]);
    let writer = issue("writer", vec!["messages:read", "media:write"]);
    let media = issue("media", vec!["media:read", "media:write"]);
    let revoked = issue("revoked", vec!["messages:read", "media:write"]);
    f.app.enable_api(true);
    let router = werc_http_api::router(Arc::new(f.app));
    let req = |method: &str, key: &str| {
        Request::builder()
            .method(method)
            .uri(format!(
                "/api/v1/messages/native/{pos}/video?account_id={}",
                f.account
            ))
            .header("Authorization", format!("Bearer {key}"))
            .header("Range", "bytes=0-15")
            .body(Body::empty())
            .unwrap()
    };
    for (method, key, status) in [
        ("GET", &reader.secret, 404),
        ("POST", &reader.secret, 403),
        ("DELETE", &reader.secret, 403),
        ("GET", &media.secret, 403),
        ("POST", &media.secret, 403),
        ("DELETE", &media.secret, 403),
    ] {
        assert_eq!(
            router
                .clone()
                .oneshot(req(method, key))
                .await
                .unwrap()
                .status(),
            status
        );
    }
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 0);
    f.provider.download_failure.store(1, Ordering::SeqCst);
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &writer.secret))
            .await
            .unwrap()
            .status(),
        415
    );
    f.provider.download_failure.store(0, Ordering::SeqCst);
    *f.provider.revoke_on_download.lock().unwrap() = Some(revoked.info.id);
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &revoked.secret))
            .await
            .unwrap()
            .status(),
        401
    );
    // A committed local cache is retained after output authorization is revoked.
    assert_eq!(
        router
            .clone()
            .oneshot(req("POST", &writer.secret))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 2);
    assert_eq!(
        router
            .clone()
            .oneshot(req("GET", &writer.secret))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router
        .clone()
        .oneshot(req("GET", &reader.secret))
        .await
        .unwrap();
    assert_eq!(response.status(), 200); // Range is explicitly unsupported, never a false 206.
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["accept-ranges"], "none");
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        include_bytes!("../../../tests/fixtures/owned-video.mp4")
    );
    *f.provider.revoke_on_decrypt.lock().unwrap() = Some(reader.info.id);
    let response = router
        .clone()
        .oneshot(req("GET", &reader.secret))
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert!(
        !response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .windows(4)
            .any(|w| w == b"ftyp")
    );
    assert_eq!(
        router
            .clone()
            .oneshot(req("DELETE", &writer.secret))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        router
            .oneshot(req("DELETE", &writer.secret))
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 2);
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn video_expiry_never_downloads_on_read_and_explicit_refresh_keeps_observation() {
    let f = Fixture::new();
    let position = video_observation(&f);
    let first = f.app.receive_video_local(&f.account, position).unwrap();
    let cursor = f.app.receive_cursor_local(&f.account).unwrap();
    f.db()
        .execute(
            "UPDATE received_video_assets SET expires_at='2000-01-01T00:00:00+00:00'",
            [],
        )
        .unwrap();
    assert!(
        f.app
            .received_video_asset_local(&f.account, position)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.app
            .received_video_content_local(&f.account, position)
            .unwrap_err()
            .code,
        "ASSET_NOT_FOUND"
    );
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 1);
    assert_eq!(f.repo.purge_expired_videos().unwrap(), 1);
    assert_eq!(f.rows().len(), 1);
    let second = f.app.receive_video_local(&f.account, position).unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.sha256, second.sha256);
    assert_eq!(f.provider.downloads.load(Ordering::SeqCst), 2);
    assert_eq!(f.app.receive_cursor_local(&f.account).unwrap(), cursor);
    assert!(f.app.sends_local(&f.account).unwrap().is_empty());
}

fn set_route(f: &Fixture, sender: &str, receiver: &str, conversation: &str) {
    *f.provider.route.lock().unwrap() = Some(ReceivedRoute {
        sender: sender.into(),
        receiver: receiver.into(),
        conversation: conversation.into(),
    });
}
#[test]
fn routed_receive_separates_conversations_with_same_native_id_and_reopens_encrypted() {
    let f = Fixture::new();
    let login = f.provider.context.lock().unwrap().sender_id.clone();
    for (sequence, conversation) in [
        (1, "owned-contact"),
        (2, "owned-group@chatroom"),
        (3, "filehelper"),
    ] {
        f.message(sequence, "4242");
        set_route(&f, conversation, &login, conversation);
        assert!(f.step().unwrap().stored);
    }
    let rows = f.rows();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|r| r.payload.schema_version == 3 && r.payload.route.is_some())
    );
    assert_ne!(rows[0].payload.id, rows[1].payload.id);
    let reopened = Application::new(
        Arc::new(SqliteRepository::open(&f.path).unwrap()),
        f.provider.clone(),
    );
    let read = reopened.received_texts_local(&f.account, 0, 100).unwrap();
    assert!(read[1].payload.route.as_ref().unwrap().valid_for(&login));
    assert_eq!(read[1].payload.target_id, "owned-group@chatroom");
    let disk = std::fs::read(&f.path).unwrap();
    for secret in ["owned-contact", "owned-group@chatroom", login.as_str()] {
        assert!(!disk.windows(secret.len()).any(|w| w == secret.as_bytes()));
    }
}
#[test]
fn routed_ack_loss_retries_once_but_changed_envelope_or_sequence_never_acknowledges() {
    let f = Fixture::new();
    let login = f.provider.context.lock().unwrap().sender_id.clone();
    f.message(1, "55");
    set_route(&f, &login, "owned-contact", "owned-contact");
    f.provider.fail_ack.store(true, Ordering::SeqCst);
    assert_eq!(f.step().err().unwrap().code, "OWNED_ACK_LOST");
    f.provider.fail_ack.store(false, Ordering::SeqCst);
    set_route(&f, "owned-contact", &login, "owned-contact");
    assert_eq!(f.step().err().unwrap().code, "RECEIVE_CONTENT_CONFLICT");
    set_route(&f, &login, "other-contact", "other-contact");
    assert_eq!(f.step().err().unwrap().code, "RECEIVE_PROTOCOL_INVALID");
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 1);
    set_route(&f, &login, "owned-contact", "owned-contact");
    let result = f.step().unwrap();
    assert!(!result.stored && result.acknowledged);
    assert_eq!(f.rows().len(), 1);
}
#[test]
fn legacy_filehelper_dedup_survives_upgrade_without_fabricated_route() {
    let f = Fixture::new();
    let login = f.provider.context.lock().unwrap().sender_id.clone();
    f.message(1, "65");
    assert!(f.step().unwrap().stored);
    let legacy = f.rows()[0].payload.id.clone();
    f.provider.context.lock().unwrap().epoch = "new-abi".into();
    f.message(1, "65");
    set_route(&f, &login, "filehelper", "filehelper");
    let result = f.step().unwrap();
    assert!(!result.stored && result.acknowledged);
    let rows = f.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].payload.id, legacy);
    assert_eq!(rows[0].payload.schema_version, 1);
    assert!(rows[0].payload.route.is_none());
    f.message(2, "66");
    assert!(f.step().unwrap().stored);
    f.message(3, "66");
    set_route(&f, "filehelper", &login, "filehelper");
    assert_eq!(f.step().err().unwrap().code, "RECEIVE_CONTENT_CONFLICT");
    assert_eq!(f.rows().len(), 2);
}
#[test]
fn unrelated_malformed_and_empty_event_routes_are_rejected_before_commit() {
    let f = Fixture::new();
    let login = f.provider.context.lock().unwrap().sender_id.clone();
    for (sender, receiver, conversation) in [
        ("a", "b", "a"),
        ("a", login.as_str(), "b"),
        ("a/b", login.as_str(), "a/b"),
        ("", login.as_str(), ""),
        ("a", login.as_str(), "a\0"),
    ] {
        f.message(1, "9");
        set_route(&f, sender, receiver, conversation);
        assert_eq!(f.step().err().unwrap().code, "RECEIVE_PROTOCOL_INVALID");
    }
    set_route(&f, "a", &login, "a");
    f.provider.pending.lock().unwrap().take();
    assert_eq!(f.step().err().unwrap().code, "RECEIVE_PROTOCOL_INVALID");
    assert_eq!(f.provider.acks.load(Ordering::SeqCst), 0);
    assert!(f.rows().is_empty());
    assert!(f.repo.receive_cursor(&f.account).unwrap().is_none());
}
#[test]
fn routed_video_keeps_login_binding_and_existing_media_transactions() {
    let f = Fixture::new();
    let login = f.provider.context.lock().unwrap().sender_id.clone();
    set_route(&f, "owned-group@chatroom", &login, "owned-group@chatroom");
    let position = video_observation(&f);
    let asset = f.app.receive_video_local(&f.account, position).unwrap();
    assert_eq!(
        f.app
            .received_video_asset_local(&f.account, position)
            .unwrap()
            .unwrap()
            .sha256,
        asset.sha256
    );
    assert_eq!(f.rows()[0].payload.target_id, "owned-group@chatroom");
}
