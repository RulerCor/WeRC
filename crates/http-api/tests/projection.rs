#![cfg(windows)]
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Instant};
use werc_application::{Application, ports::CapabilityProvider};
use werc_domain::{AppResult, browse::*, history::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

struct Generated {
    conversations: usize,
    per_conversation: usize,
}
impl CapabilityProvider for Generated {
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        vec![]
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.unprotect_key(c, k)
    }
    fn stream_database(
        &self,
        request: &HistoryReadRequest,
        visit: &mut dyn FnMut(HistoryReadEvent) -> AppResult<()>,
    ) -> AppResult<()> {
        let interleaved = request.path.contains("interleaved");
        let count = self.conversations * self.per_conversation;
        let template = HistoryBundle {
            schema: "synthetic-projection/v1".into(),
            source_sha256: "3".repeat(64),
            encryption: "synthetic".into(),
            wal_committed_frames: 0,
            messages: vec![],
            contacts: vec![],
            sessions: vec![],
            warnings: vec![],
        };
        visit(HistoryReadEvent::Begin {
            schema: template.schema.clone(),
            source_sha256: template.source_sha256.clone(),
            encryption: template.encryption.clone(),
            wal_committed_frames: 0,
        })?;
        let mut batches = 0;
        // 留出联系人记录的批次名额，总行数仍不超过500。
        for start in (0..count).step_by(200) {
            let mut data = template.clone();
            for index in start..(start + 200).min(count) {
                let conversation = if interleaved {
                    index % self.conversations
                } else {
                    index / self.per_conversation
                };
                let id = format!("synthetic-private-peer-{conversation:05}");
                if if interleaved {
                    index < self.conversations
                } else {
                    index % self.per_conversation == 0
                } {
                    data.contacts.push(HistoryContact {
                        username: id.clone(),
                        nickname: Some(format!("synthetic-private-name-{conversation}")),
                        remark: None,
                        alias: None,
                    });
                }
                data.messages.push(HistoryMessage {
                    id: index.to_string(),
                    conversation_id: id.clone(),
                    sender_id: Some(id),
                    local_id: index as i64,
                    server_id: (i64::MAX - index as i64).to_string(),
                    timestamp: 1700000000 + (index / self.conversations) as i64,
                    sequence: index as i64,
                    message_type: 1,
                    text: Some(format!("synthetic-private-body-{index}")),
                    content_status: "decoded".into(),
                });
            }
            let digest = hex::encode(Sha256::digest(serde_json::to_vec(&data).unwrap()));
            visit(HistoryReadEvent::Batch {
                sequence: batches,
                digest,
                data,
            })?;
            batches += 1;
        }
        visit(HistoryReadEvent::End {
            batches,
            message_count: count,
            contact_count: self.conversations,
            session_count: 0,
        })
    }
}
fn request(conversation: usize) -> BrowseRequest {
    BrowseRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        mode: ReadMode::ImportedSnapshot,
        source_id: None,
        direction: BrowseDirection::Latest,
        anchor_id: None,
        anchor_timestamp: None,
        query: HistoryQuery {
            conversation_id: Some(format!("synthetic-private-peer-{conversation:05}")),
            sender_id: None,
            message_type: None,
            search: None,
            from: None,
            until: None,
            cursor: None,
            limit: 60,
        },
    }
}
struct Fixture {
    root: PathBuf,
    path: PathBuf,
    app: Application,
    account: String,
    source: HistorySource,
}
impl Fixture {
    fn new(conversations: usize, per_conversation: usize) -> Self {
        Self::layout(conversations, per_conversation, false)
    }
    fn layout(conversations: usize, per_conversation: usize, interleaved: bool) -> Self {
        let root = std::env::temp_dir().join(format!("werc-projection-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("state.sqlite3");
        let app = Application::new(
            Arc::new(SqliteRepository::open(&path).unwrap()),
            Arc::new(Generated {
                conversations,
                per_conversation,
            }),
        );
        let account = app
            .register_account_local("synthetic projection")
            .unwrap()
            .id;
        let source = app
            .import_history_local(
                &account,
                HistoryReadRequest {
                    path: if interleaved {
                        "synthetic-interleaved".into()
                    } else {
                        "synthetic-generated".into()
                    },
                    key_hex: None,
                    allow_plaintext: true,
                },
                true,
            )
            .unwrap();
        Self {
            root,
            path,
            app,
            account,
            source,
        }
    }
}
#[test]
fn projection_write_batches_are_atomic_bounded_and_hidden_until_published() {
    use werc_application::ports::Repository;
    use werc_domain::projection::{ProtectedProjection, ProtectedProjectionRow};
    let repo = SqliteRepository::in_memory().unwrap();
    let account = repo
        .register_account("synthetic projection batch")
        .unwrap()
        .id;
    let data = ProtectedProjection {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account.clone(),
        generation: "a".repeat(64),
        wrapped_key: vec![1],
        nonce: vec![0; 12],
        ciphertext: vec![2; 16],
    };
    repo.begin_projection(&data).unwrap();
    let make = |n: usize| ProtectedProjectionRow {
        token: format!("{n:064x}"),
        nonce: vec![0; 12],
        ciphertext: vec![3; 16],
    };
    assert!(repo.append_projection(&account, &data.id, &[]).is_err());
    assert!(
        repo.append_projection(&account, &data.id, &(0..65).map(make).collect::<Vec<_>>())
            .is_err()
    );
    // 第二行冲突时，第一行和payload计数必须一并回滚。
    assert!(
        repo.append_projection(&account, &data.id, &[make(0), make(0)])
            .is_err()
    );
    repo.append_projection(&account, &data.id, &(0..64).map(make).collect::<Vec<_>>())
        .unwrap();
    assert!(
        repo.projection(&account, &data.generation)
            .unwrap()
            .is_none()
    );
    assert!(
        repo.projection_row(&account, &data.id, &format!("{:064x}", 0))
            .unwrap()
            .is_none()
    );
    repo.finish_projection(&data).unwrap();
    assert!(
        repo.projection(&account, &data.generation)
            .unwrap()
            .is_some()
    );
    for n in 0..64 {
        assert!(
            repo.projection_row(&account, &data.id, &format!("{n:064x}"))
                .unwrap()
                .is_some()
        );
    }
    assert!(
        repo.append_projection(&account, &data.id, &[make(64)])
            .is_err()
    );
    repo.remove_projection(&account, &data.id).unwrap();
    assert!(
        repo.projection_row(&account, &data.id, &format!("{:064x}", 0))
            .unwrap()
            .is_none()
    );
}
#[test]
fn oversized_projection_blobs_are_rejected_before_rust_payload_copy_and_rebuild() {
    use werc_application::ports::Repository;
    use werc_domain::projection::{PROJECTION_HEADER_BYTES, PROJECTION_ROW_BYTES};
    let f = Fixture::new(1, 10);
    let repo = SqliteRepository::open(&f.path).unwrap();
    let c = Connection::open(&f.path).unwrap();
    for (table, column, bytes) in [
        ("history_projections", "wrapped_key", 16385),
        ("history_projections", "nonce", 13),
        (
            "history_projections",
            "ciphertext",
            PROJECTION_HEADER_BYTES + 17,
        ),
        ("history_projection_rows", "nonce", 13),
        (
            "history_projection_rows",
            "ciphertext",
            PROJECTION_ROW_BYTES + 17,
        ),
    ] {
        assert_eq!(
            f.app
                .browse_messages_local(&f.account, &request(0))
                .unwrap()
                .data
                .len(),
            10
        );
        c.execute(
            &format!("UPDATE {table} SET {column}=zeroblob(?1)"),
            [bytes as i64],
        )
        .unwrap();
        let (id, generation): (String, String) = c
            .query_row(
                "SELECT id,generation FROM history_projections WHERE ready=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        // SQLite CASE返回空的无效保护材料，而不是把越界BLOB交给Rust。
        if table == "history_projections" {
            let value = repo.projection(&f.account, &generation).unwrap().unwrap();
            let field = match column {
                "wrapped_key" => value.wrapped_key,
                "nonce" => value.nonce,
                _ => value.ciphertext,
            };
            assert!(field.is_empty());
        } else {
            let token: String = c
                .query_row("SELECT token FROM history_projection_rows", [], |r| {
                    r.get(0)
                })
                .unwrap();
            let value = repo
                .projection_row(&f.account, &id, &token)
                .unwrap()
                .unwrap();
            assert!(if column == "nonce" {
                value.nonce.is_empty()
            } else {
                value.ciphertext.is_empty()
            });
        }
        assert_eq!(
            f.app
                .browse_messages_local(&f.account, &request(0))
                .unwrap_err()
                .code,
            "PROJECTION_CORRUPTED"
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM history_projections", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(f.app.history_sources_local(&f.account).unwrap().len(), 1);
    assert_eq!(
        f.app
            .browse_messages_local(&f.account, &request(0))
            .unwrap()
            .data
            .len(),
        10
    );
    drop(c);
    drop(repo);
    let root = f.root.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn persistent_projection_authenticates_rows_and_rebuilds_without_plaintext_indexes() {
    let f = Fixture::new(10, 120);
    let first = f
        .app
        .browse_messages_local(&f.account, &request(0))
        .unwrap();
    assert_eq!(first.context.query_strategy, "protected_batch_projection");
    assert!(
        first
            .context
            .query_metrics
            .as_ref()
            .unwrap()
            .projection_built
    );
    assert_eq!(first.data.len(), 60);
    let reopened = Application::new(
        Arc::new(SqliteRepository::open(&f.path).unwrap()),
        Arc::new(Generated {
            conversations: 10,
            per_conversation: 120,
        }),
    );
    let warm = reopened
        .browse_messages_local(&f.account, &request(0))
        .unwrap();
    let metrics = warm.context.query_metrics.unwrap();
    assert!(!metrics.projection_built);
    assert_eq!(metrics.authenticated_batches, 1);
    assert_eq!(warm.data[0].id, first.data[0].id);
    let c = Connection::open(&f.path).unwrap();
    let (token, cipher): (String, Vec<u8>) = c
        .query_row(
            "SELECT token,ciphertext FROM history_projection_rows LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(token.len(), 64);
    assert!(!String::from_utf8_lossy(&cipher).contains("synthetic-private"));
    for column in ["nonce", "ciphertext"] {
        c.execute(
            &format!("UPDATE history_projections SET {column}=zeroblob(length({column}))"),
            [],
        )
        .unwrap();
        assert_eq!(
            reopened
                .browse_messages_local(&f.account, &request(0))
                .unwrap_err()
                .code,
            "PROJECTION_CORRUPTED"
        );
        let rebuilt = reopened
            .browse_messages_local(&f.account, &request(0))
            .unwrap();
        assert!(rebuilt.context.query_metrics.unwrap().projection_built);
    }
    // 删除定位行不能静默少消息；清理坏缓存后下一次重新认证原来源。
    c.execute("DELETE FROM history_projection_rows", [])
        .unwrap();
    assert_eq!(
        reopened
            .browse_messages_local(&f.account, &request(0))
            .unwrap_err()
            .code,
        "PROJECTION_CORRUPTED"
    );
    assert_eq!(
        reopened
            .browse_messages_local(&f.account, &request(0))
            .unwrap()
            .data
            .len(),
        60
    );
    let old = f
        .app
        .browse_messages_local(&f.account, &request(0))
        .unwrap();
    let mut older = request(0);
    older.direction = BrowseDirection::Older;
    older.query.cursor = old.older_cursor;
    assert_eq!(
        f.app
            .browse_messages_local(&f.account, &older)
            .unwrap()
            .data
            .len(),
        60
    );
    // 目标批次的原AEAD仍不可绕过；投影不会信任损坏的原消息。
    c.execute("UPDATE history_batches SET ciphertext=zeroblob(length(ciphertext)) WHERE source_id=?1 AND sequence=0",[&f.source.id]).unwrap();
    assert_eq!(
        f.app
            .browse_messages_local(&f.account, &request(0))
            .unwrap_err()
            .code,
        "SOURCE_CORRUPTED"
    );
    drop(c);
    drop(reopened);
    let root = f.root.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn clearing_only_published_projection_preserves_archive_and_rebuilds_on_demand() {
    use werc_application::ports::Repository;
    use werc_domain::projection::ProtectedProjection;
    let f = Fixture::new(3, 100);
    assert_eq!(f.app.clear_browse_cache_local(&f.account).unwrap(), 0);
    let original = f.app.history_sources_local(&f.account).unwrap();
    let manifest = f
        .app
        .browse_messages_local(&f.account, &request(0))
        .unwrap();
    assert_eq!(manifest.data.len(), 60);
    let repo = SqliteRepository::open(&f.path).unwrap();
    let payload = repo
        .history_payload(&f.account, &f.source.id)
        .unwrap()
        .ciphertext;
    assert_eq!(repo.clear_projections("another-scope").unwrap(), 0);
    let pending = ProtectedProjection {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: f.account.clone(),
        generation: "b".repeat(64),
        wrapped_key: vec![1],
        nonce: vec![],
        ciphertext: vec![],
    };
    repo.begin_projection(&pending).unwrap();
    assert_eq!(f.app.clear_browse_cache_local(&f.account).unwrap(), 1);
    let c = Connection::open(&f.path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM history_projection_rows", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM history_projections WHERE ready=0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        serde_json::to_value(f.app.history_sources_local(&f.account).unwrap()).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    assert_eq!(
        repo.history_payload(&f.account, &f.source.id)
            .unwrap()
            .ciphertext,
        payload
    );
    let rebuilt = f
        .app
        .browse_messages_local(&f.account, &request(0))
        .unwrap();
    assert!(rebuilt.context.query_metrics.unwrap().projection_built);
    assert_eq!(rebuilt.data.len(), 60);
    repo.remove_projection(&f.account, &pending.id).unwrap();
    drop(c);
    drop(repo);
    let root = f.root.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn projection_cache_is_generation_bound_and_precancel_keeps_it_absent() {
    let f = Fixture::new(4, 150);
    let query = request(0);
    f.app.cancel_browse_local(&query.request_id).unwrap();
    assert_eq!(
        f.app
            .browse_messages_local(&f.account, &query)
            .unwrap_err()
            .code,
        "QUERY_CANCELED"
    );
    let c = Connection::open(&f.path).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM history_projections", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let _ = f
        .app
        .browse_messages_local(&f.account, &request(0))
        .unwrap();
    let total: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM history_projections WHERE ready=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(total, 1);
    // 保护清单被替换后不能命中旧投影；完整重建认证必须失败。
    c.execute(
        "UPDATE history_sources SET ciphertext=zeroblob(length(ciphertext)) WHERE id=?1",
        [&f.source.id],
    )
    .unwrap();
    assert_eq!(
        f.app
            .browse_messages_local(&f.account, &request(0))
            .unwrap_err()
            .code,
        "SOURCE_CORRUPTED"
    );
    drop(c);
    let root = f.root.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}
struct GatedProvider {
    generated: Generated,
    entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}
impl GatedProvider {
    fn release(&self) {
        *self.released.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake.notify_all();
    }
}
struct Release(Arc<GatedProvider>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
impl CapabilityProvider for GatedProvider {
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        vec![]
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        self.generated.protect_key(c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> AppResult<Vec<u8>> {
        if c.starts_with("WeRC/history/v2/")
            && let Some(entered) = self.entered.lock().unwrap().take()
        {
            entered.send(()).unwrap();
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.wake.wait(released).unwrap();
            }
        }
        self.generated.unprotect_key(c, k)
    }
}
#[test]
fn cancellation_during_cold_projection_releases_no_staged_result_within_two_seconds() {
    let f = Fixture::new(4, 150);
    let repo = Arc::new(SqliteRepository::open(&f.path).unwrap());
    let (sender, receiver) = std::sync::mpsc::channel();
    let provider = Arc::new(GatedProvider {
        generated: Generated {
            conversations: 4,
            per_conversation: 150,
        },
        entered: std::sync::Mutex::new(Some(sender)),
        released: std::sync::Mutex::new(false),
        wake: std::sync::Condvar::new(),
    });
    let _release = Release(provider.clone());
    let app = Arc::new(Application::new(repo, provider.clone()));
    let request = request(0);
    let id = request.request_id.clone();
    let worker = {
        let app = app.clone();
        let account = f.account.clone();
        std::thread::spawn(move || app.browse_messages_local(&account, &request))
    };
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        app.clear_browse_cache_local(&f.account).unwrap_err().code,
        "BUSY"
    );
    let start = Instant::now();
    app.cancel_browse_local(&id).unwrap();
    provider.release();
    assert_eq!(worker.join().unwrap().unwrap_err().code, "QUERY_CANCELED");
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(2));
    let c = Connection::open(&f.path).unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM history_projections", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    println!("cold_projection_cancel_ms={}", elapsed.as_millis());
    drop(c);
    drop(app);
    let root = f.root.clone();
    drop(f);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "百万消息独立规模验收；用release显式运行，生成分批数据并记录实测指标"]
fn million_messages_ten_thousand_conversations_projection_benchmark() {
    for interleaved in [false, true] {
        let start = Instant::now();
        let f = Fixture::layout(10_000, 100, interleaved);
        let import_ms = start.elapsed().as_millis();
        let cold_start = Instant::now();
        let mut query = request(0);
        query.query.conversation_id = None;
        let first = f
            .app
            .browse_conversations_local(&f.account, &query)
            .unwrap();
        let cold_ms = cold_start.elapsed().as_millis();
        assert_eq!(first.data.len(), 60);
        assert_eq!(
            first
                .context
                .query_metrics
                .as_ref()
                .unwrap()
                .authenticated_batches,
            5000
        );
        let mut timings = Vec::new();
        let mut scans = Vec::new();
        for n in 0..30 {
            let start = Instant::now();
            let page = f
                .app
                .browse_messages_local(&f.account, &request(n * 337 % 10_000))
                .unwrap();
            timings.push(start.elapsed().as_secs_f64() * 1000.);
            scans.push(page.context.query_metrics.unwrap().authenticated_batches);
            assert_eq!(page.data.len(), 60);
            assert!(page.older_cursor.is_some());
            let mut older = request(n * 337 % 10_000);
            older.direction = BrowseDirection::Older;
            older.query.cursor = page.older_cursor;
            let start = Instant::now();
            let previous = f.app.browse_messages_local(&f.account, &older).unwrap();
            timings.push(start.elapsed().as_secs_f64() * 1000.);
            assert_eq!(previous.data.len(), 40);
        }
        timings.sort_by(f64::total_cmp);
        let p95 = timings[(timings.len() * 95).div_ceil(100) - 1];
        let memory = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id()),
            ])
            .output()
            .unwrap();
        let peak = String::from_utf8_lossy(&memory.stdout)
            .trim()
            .parse::<u64>()
            .unwrap();
        let c = Connection::open(&f.path).unwrap();
        let projection_bytes: i64 = c
            .query_row(
                "SELECT SUM(payload_bytes) FROM history_projections",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let report = serde_json::json!({"fixture":if interleaved {"synthetic-interleaved"} else {"synthetic-grouped-by-conversation"},"conversations":10000,"messages":1000000,"batch_messages":200,"import_ms":import_ms,"cold_prepare_ms":cold_ms,"hot_samples":timings.len(),"hot_message_and_older_p95_ms":p95,"message_batches_read_per_hot_query":scans,"peak_process_working_set_bytes":peak,"projection_bytes":projection_bytes,"target_p95_ms":500,"passed_hot_target":p95<=500.,"real_wechat_verified":false,"state_path":f.path});
        println!("{report}");
        let report_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if interleaved {
            "../../docs/verification/projection-scale-interleaved.json"
        } else {
            "../../docs/verification/projection-scale.json"
        });
        std::fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        drop(c);
        let root = f.root.clone();
        drop(f);
        std::fs::remove_dir_all(root).unwrap();
        assert!(p95 <= 500., "实测热页p95超500ms目标：{p95}ms");
    }
}
