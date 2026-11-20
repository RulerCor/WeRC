use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{
    AppError, AppResult,
    capability::Capability,
    connection::{AcquiredDatabaseKey, ClientInstallation},
    history::{HistoryBundle, HistoryReadRequest},
    onboarding::SetupRequest,
};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;
struct Provider {
    gate: Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>,
    ready: AtomicBool,
    fail: AtomicBool,
}
impl CapabilityProvider for Provider {
    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
    fn inspect(&self, path: &str) -> AppResult<ClientInstallation> {
        Ok(ClientInstallation {
            id: path.into(),
            executable_path: path.into(),
            version: if path == "synthetic-4.2.exe" { "4.2.0.52" } else { "4.1.11.65535" }.into(),
            architecture: "x86_64".into(),
            product_name: "synthetic".into(),
            sha256: "1".repeat(64),
            process_ids: vec![42],
            sources: vec!["synthetic".into()],
            compatibility: "unsupported".into(),
            reason: "synthetic".into(),
        })
    }
    fn offline_ready(&self, _: &str) -> AppResult<bool> {
        Ok(self.ready.load(Ordering::SeqCst))
    }
    fn acquire_database_key(
        &self,
        _: &ClientInstallation,
        _: u32,
        path: &str,
    ) -> AppResult<AcquiredDatabaseKey> {
        if let Some((entered, release)) = &self.gate {
            entered.wait();
            release.wait();
        }
        if path.ends_with("incomplete-key.db") {
            return Err(AppError::new(
                "KEY_SCAN_INCOMPLETE",
                "synthetic scan budget",
            ));
        }
        if self.fail.load(Ordering::SeqCst) || path.ends_with("missing-key.db") {
            return Err(AppError::new("KEY_NOT_FOUND", "synthetic"));
        }
        Ok(AcquiredDatabaseKey {
            key: zeroize::Zeroizing::new([0x5a; 32]),
            salt: [1; 16],
            first_page_sha256: [2; 32],
        })
    }
    fn database_salt(&self, _: &str) -> AppResult<[u8; 16]> {
        Ok([1; 16])
    }
    fn protect_key(&self, c: &str, b: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(c, b)
    }
    fn unprotect_key(&self, c: &str, b: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.unprotect_key(c, b)
    }
    fn read_database(&self, r: &HistoryReadRequest) -> AppResult<HistoryBundle> {
        assert_eq!(r.key_hex.as_deref(), Some("5a".repeat(32).as_str()));
        Ok(HistoryBundle {
            schema: "synthetic".into(),
            source_sha256: "2".repeat(64),
            encryption: "synthetic".into(),
            wal_committed_frames: 0,
            messages: vec![],
            contacts: vec![],
            sessions: vec![],
            warnings: vec![],
        })
    }
}
fn fixture() -> (std::path::PathBuf, SetupRequest) {
    let root = std::env::temp_dir().join(format!("werc-setup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let db = root.join("session.db");
    std::fs::write(&db, [0x35u8; 4096]).unwrap();
    (
        root,
        SetupRequest {
            label: "合成引导".into(),
            executable: "synthetic.exe".into(),
            process_id: 42,
            databases: vec![db.to_str().unwrap().into()],
        },
    )
}
#[test]
fn setup_accepts_4_2_read_only_keys_without_claiming_native_readiness() {
    let (root, mut request) = fixture();
    request.executable = "synthetic-4.2.exe".into();
    let store = Arc::new(SqliteRepository::open(&root.join("state.db")).unwrap());
    let app = Application::new(store.clone(), Arc::new(Provider {
        gate: None,
        ready: AtomicBool::new(false),
        fail: AtomicBool::new(false),
    }));
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    assert_eq!(run.binding.installation.version, "4.2.0.52");
    assert_eq!(run.binding.validation, "metadata_only");
    let status = app.setup_step_local(&run.id).unwrap();
    assert!(status.run.unwrap().items[0].key_ready);
    drop(app);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn setup_is_protected_resumable_and_waits_for_offline_sources() {
    let (root, request) = fixture();
    let store = Arc::new(SqliteRepository::open(&root.join("state.db")).unwrap());
    let provider = Arc::new(Provider {
        gate: None,
        ready: AtomicBool::new(false),
        fail: AtomicBool::new(false),
    });
    let app = Application::new(store.clone(), provider.clone());
    assert_eq!(
        app.setup_start_local(request.clone(), false)
            .err()
            .unwrap()
            .code,
        "CONSENT_REQUIRED"
    );
    assert!(app.accounts_local().unwrap().is_empty());
    let status = app.setup_start_local(request.clone(), true).unwrap();
    let run = status.run.unwrap();
    assert_eq!(
        app.setup_start_local(request.clone(), true)
            .err()
            .unwrap()
            .code,
        "SETUP_EXISTS"
    );
    assert_eq!(app.accounts_local().unwrap().len(), 1);
    let payload = store.setup_payload().unwrap().unwrap();
    assert!(
        !payload
            .windows(request.databases[0].len())
            .any(|w| w == request.databases[0].as_bytes())
    );
    assert!(app.setup_step_local(&run.id).unwrap().run.unwrap().items[0].key_ready);
    assert_eq!(
        app.setup_step_local(&run.id).unwrap().run.unwrap().phase,
        "waiting"
    );
    assert!(
        app.history_sources_local(&run.binding.account_id)
            .unwrap()
            .is_empty()
    );
    drop(app);
    drop(store);
    let store = Arc::new(SqliteRepository::open(&root.join("state.db")).unwrap());
    let app = Application::new(store.clone(), provider.clone());
    assert!(!app.setup_status_local().unwrap().active);
    assert_eq!(
        app.setup_status_local().unwrap().run.unwrap().items[0]
            .outcome
            .as_deref(),
        Some("pending")
    );
    assert_eq!(
        app.setup_step_local(&run.id).err().unwrap().code,
        "CONSENT_REQUIRED"
    );
    assert_eq!(
        app.setup_resume_local(&run.id, false, false)
            .err()
            .unwrap()
            .code,
        "CONSENT_REQUIRED"
    );
    app.setup_resume_local(&run.id, true, false).unwrap();
    provider.ready.store(true, Ordering::SeqCst);
    let complete = app.setup_step_local(&run.id).unwrap();
    assert!(!complete.active);
    assert_eq!(complete.run.unwrap().phase, "complete");
    assert_eq!(
        app.history_sources_local(&run.binding.account_id)
            .unwrap()
            .len(),
        1
    );
    app.setup_reset_local().unwrap();
    app.setup_start_local(request, true).unwrap();
    assert_eq!(app.accounts_local().unwrap().len(), 1);
    app.setup_pause_local().unwrap();
    drop(app);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn setup_missing_key_is_partial_and_external_key_change_does_not_fake_success() {
    let (root, request) = fixture();
    let store = Arc::new(SqliteRepository::in_memory().unwrap());
    let provider = Arc::new(Provider {
        gate: None,
        ready: AtomicBool::new(true),
        fail: AtomicBool::new(true),
    });
    let app = Application::new(store.clone(), provider);
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    let failed = app.setup_step_local(&run.id).unwrap();
    assert!(!failed.active);
    let deferred = failed.run.unwrap();
    assert_eq!(deferred.phase, "partial");
    assert_eq!(deferred.items[0].error.as_deref(), Some("KEY_NOT_FOUND"));
    assert_eq!(deferred.items[0].outcome.as_deref(), Some("key_deferred"));
    assert!(deferred.items[0].skipped);
    assert!(deferred.items[0].source.is_none());
    assert!(
        app.history_sources_local(&run.binding.account_id)
            .unwrap()
            .is_empty()
    );
    app.remove_key_local(&run.binding.account_id).unwrap();
    assert_eq!(
        app.setup_resume_local(&run.id, true, false)
            .err()
            .unwrap()
            .code,
        "CONNECTION_CHANGED"
    );
    let mut bytes = store.setup_payload().unwrap().unwrap();
    bytes[0] ^= 1;
    store.save_setup_payload(Some(&bytes)).unwrap();
    assert!(app.setup_status_local().is_err());
    app.setup_reset_local().unwrap();
    assert!(app.setup_status_local().unwrap().run.is_none());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn pause_during_acquisition_stops_next_step_and_rejects_reset_until_settled() {
    let (root, request) = fixture();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let provider = Arc::new(Provider {
        ready: AtomicBool::new(true),
        fail: AtomicBool::new(false),
        gate: Some((entered.clone(), release.clone())),
    });
    let app = Arc::new(Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        provider,
    ));
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    let id = run.id.clone();
    let worker = app.clone();
    let task = std::thread::spawn(move || worker.setup_step_local(&id));
    entered.wait();
    app.setup_pause_local().unwrap();
    let reset = app.setup_reset_local();
    let status = app.setup_status_local();
    release.wait();
    let result = task.join().unwrap().unwrap();
    assert_eq!(reset.err().unwrap().code, "BUSY");
    assert!(status.unwrap().working);
    assert!(!result.active);
    assert!(result.run.unwrap().items[0].key_ready);
    assert_eq!(
        app.setup_step_local(&run.id).err().unwrap().code,
        "CONSENT_REQUIRED"
    );
    assert!(
        app.history_sources_local(&run.binding.account_id)
            .unwrap()
            .is_empty()
    );
    std::fs::remove_dir_all(root).unwrap();
}

struct ScenarioProvider {
    base: Provider,
    offline_error: std::sync::Mutex<Option<(String, String)>>,
    read_busy: AtomicBool,
}
impl ScenarioProvider {
    fn new() -> Self {
        Self {
            base: Provider {
                gate: None,
                ready: AtomicBool::new(true),
                fail: AtomicBool::new(false),
            },
            offline_error: std::sync::Mutex::new(None),
            read_busy: AtomicBool::new(false),
        }
    }
}
impl CapabilityProvider for ScenarioProvider {
    fn capabilities(&self) -> Vec<Capability> {
        self.base.capabilities()
    }
    fn inspect(&self, path: &str) -> AppResult<ClientInstallation> {
        self.base.inspect(path)
    }
    fn offline_ready(&self, path: &str) -> AppResult<bool> {
        if let Some((name, code)) = self.offline_error.lock().unwrap().as_ref()
            && std::path::Path::new(path).file_name().unwrap().to_str() == Some(name.as_str())
        {
            return Err(AppError::new(code, "synthetic per-file offline error"));
        }
        self.base.offline_ready(path)
    }
    fn acquire_database_key(
        &self,
        i: &ClientInstallation,
        pid: u32,
        path: &str,
    ) -> AppResult<AcquiredDatabaseKey> {
        self.base.acquire_database_key(i, pid, path)
    }
    fn database_salt(&self, path: &str) -> AppResult<[u8; 16]> {
        self.base.database_salt(path)
    }
    fn protect_key(&self, c: &str, b: &[u8]) -> AppResult<Vec<u8>> {
        self.base.protect_key(c, b)
    }
    fn unprotect_key(&self, c: &str, b: &[u8]) -> AppResult<Vec<u8>> {
        self.base.unprotect_key(c, b)
    }
    fn read_database(&self, r: &HistoryReadRequest) -> AppResult<HistoryBundle> {
        if self.read_busy.load(Ordering::SeqCst) {
            return Err(AppError::new("BUSY", "synthetic importer lock"));
        }
        if r.path.contains("unsupported.db") {
            return Err(AppError::new(
                "SCHEMA_UNSUPPORTED",
                "synthetic unknown schema",
            ));
        }
        if r.path.contains("authfailed.db") {
            return Err(AppError::new(
                "KEY_OR_PAGE_AUTHENTICATION_FAILED",
                "synthetic authentication failure",
            ));
        }
        let mut bundle = self.base.read_database(r)?;
        use sha2::{Digest, Sha256};
        bundle.source_sha256 = hex::encode(Sha256::digest(r.path.as_bytes()));
        Ok(bundle)
    }
}

fn plan_files(root: &std::path::Path, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .map(|name| {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, [0x35; 4096]).unwrap();
            path.to_str().unwrap().into()
        })
        .collect()
}
fn settle(app: &Application, id: &str) -> werc_domain::onboarding::SetupRun {
    for _ in 0..1100 {
        let status = app.setup_step_local(id).unwrap();
        if !status.active {
            return status.run.unwrap();
        }
    }
    panic!("setup did not settle within its bounded plan");
}

#[test]
fn twenty_four_sources_include_large_database_without_rejecting_the_plan() {
    let (root, mut request) = fixture();
    let names: Vec<String> = (0..24)
        .map(|i| format!("db_storage/message/message_{i}.db"))
        .collect();
    request.databases = plan_files(&root, &names.iter().map(String::as_str).collect::<Vec<_>>());
    std::fs::OpenOptions::new()
        .write(true)
        .open(&request.databases[7])
        .unwrap()
        .set_len(257 * 1024 * 1024)
        .unwrap();
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(ScenarioProvider::new()),
    );
    let run = app
        .setup_start_local(request.clone(), true)
        .unwrap()
        .run
        .unwrap();
    assert_eq!(run.items.len(), 24);
    assert!(run.items[7].skipped);
    assert_eq!(run.items[7].outcome.as_deref(), Some("resource_deferred"));
    assert_eq!(run.items[7].error.as_deref(), Some("SOURCE_LIMIT_EXCEEDED"));
    let run = settle(&app, &run.id);
    assert_eq!(run.phase, "partial");
    assert_eq!(run.items.iter().filter(|i| i.source.is_some()).count(), 23);
    assert!(!run.items[7].key_ready);
    assert_eq!(
        std::fs::metadata(&request.databases[7]).unwrap().len(),
        257 * 1024 * 1024
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_schema_does_not_stop_later_supported_sources_or_become_retryable() {
    let (root, mut request) = fixture();
    request.databases = plan_files(&root, &["first.db", "unsupported.db", "last.db"]);
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(ScenarioProvider::new()),
    );
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    let run = settle(&app, &run.id);
    assert_eq!(run.phase, "partial");
    assert_eq!(run.items[0].outcome.as_deref(), Some("imported"));
    assert_eq!(run.items[1].outcome.as_deref(), Some("unsupported"));
    assert_eq!(run.items[1].error.as_deref(), Some("SCHEMA_UNSUPPORTED"));
    assert!(run.items[1].skipped);
    assert!(run.items[1].source.is_none());
    assert_eq!(run.items[2].outcome.as_deref(), Some("imported"));
    app.setup_resume_local(&run.id, true, false).unwrap();
    let repeated = settle(&app, &run.id);
    assert!(repeated.items[1].skipped);
    assert_eq!(repeated.items[1].error, run.items[1].error);
    assert_eq!(repeated.phase, "partial");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn later_offline_error_is_attributed_to_that_file_and_busy_is_not_failure() {
    let (root, mut request) = fixture();
    request.databases = plan_files(&root, &["first.db", "later.db"]);
    let provider = Arc::new(ScenarioProvider::new());
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        provider.clone(),
    );
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    app.setup_step_local(&run.id).unwrap();
    app.setup_step_local(&run.id).unwrap();
    *provider.offline_error.lock().unwrap() = Some(("later.db".into(), "UNSAFE_PATH".into()));
    let failed = app.setup_step_local(&run.id).unwrap();
    assert!(!failed.active);
    let failed = failed.run.unwrap();
    assert!(failed.items[0].error.is_none());
    assert_eq!(failed.items[1].error.as_deref(), Some("UNSAFE_PATH"));
    assert_eq!(failed.items[1].outcome.as_deref(), Some("failed"));
    *provider.offline_error.lock().unwrap() = None;
    app.setup_resume_local(&run.id, true, false).unwrap();
    provider.read_busy.store(true, Ordering::SeqCst);
    let waiting = app.setup_step_local(&run.id).unwrap();
    assert!(waiting.active);
    assert_eq!(waiting.run.as_ref().unwrap().phase, "waiting");
    assert!(waiting.run.unwrap().items.iter().all(|i| i.error.is_none()));
    provider.read_busy.store(false, Ordering::SeqCst);
    assert_eq!(settle(&app, &run.id).phase, "complete");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_source_reuses_manual_scope_and_reset_preserves_history_and_keys() {
    let (root, request) = fixture();
    let (other_root, other_request) = fixture();
    let store = Arc::new(SqliteRepository::in_memory().unwrap());
    let legacy = store.setup_account("legacy singleton").unwrap();
    let app = Application::new(store.clone(), Arc::new(ScenarioProvider::new()));
    let first = app
        .setup_start_local(request.clone(), true)
        .unwrap()
        .run
        .unwrap();
    assert_eq!(first.binding.account_id, legacy.id);
    let first = settle(&app, &first.id);
    let revision = store.key_revision(&first.binding.account_id).unwrap();
    assert_eq!(
        app.history_sources_local(&first.binding.account_id)
            .unwrap()
            .len(),
        1
    );
    app.setup_reset_local().unwrap();
    assert_eq!(
        store.key_revision(&first.binding.account_id).unwrap(),
        revision
    );
    assert!(
        store
            .protected_key(&first.binding.account_id)
            .unwrap()
            .is_some()
    );
    let repeated = app.setup_start_local(request, true).unwrap().run.unwrap();
    assert_eq!(repeated.binding.account_id, first.binding.account_id);
    let repeated = settle(&app, &repeated.id);
    assert_eq!(
        repeated.items[0].outcome.as_deref(),
        Some("already_imported")
    );
    app.setup_reset_local().unwrap();
    assert_eq!(
        app.setup_start_local(other_request, true)
            .err()
            .unwrap()
            .code,
        "SOURCE_CONFLICT"
    );
    assert_eq!(app.accounts_local().unwrap().len(), 1);
    assert_eq!(
        app.history_sources_local(&first.binding.account_id)
            .unwrap()
            .len(),
        1
    );
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(other_root).unwrap();
}

#[test]
fn mixed_roots_and_case_duplicate_paths_are_rejected_before_registration() {
    let (root, mut request) = fixture();
    let (other_root, other_request) = fixture();
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(ScenarioProvider::new()),
    );
    request.databases.push(other_request.databases[0].clone());
    assert_eq!(
        app.setup_start_local(request.clone(), true)
            .err()
            .unwrap()
            .code,
        "SOURCE_ROOT_MISMATCH"
    );
    request.databases[1] = request.databases[0].to_uppercase();
    assert_eq!(
        app.setup_start_local(request, true).err().unwrap().code,
        "INVALID_ARGUMENT"
    );
    assert!(app.accounts_local().unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(other_root).unwrap();
}

#[test]
fn five_hundred_twelve_item_state_has_a_bounded_but_sufficient_encrypted_budget() {
    let (root, mut request) = fixture();
    let names: Vec<String> = (0..512)
        .map(|i| format!("db_storage/message/message_{i}.db"))
        .collect();
    request.databases = plan_files(&root, &names.iter().map(String::as_str).collect::<Vec<_>>());
    let store = Arc::new(SqliteRepository::in_memory().unwrap());
    let app = Application::new(store.clone(), Arc::new(ScenarioProvider::new()));
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    assert_eq!(run.items.len(), 512);
    assert!(store.setup_payload().unwrap().unwrap().len() > 262144);
    drop(app);
    let app = Application::new(store.clone(), Arc::new(ScenarioProvider::new()));
    let restored = app.setup_status_local().unwrap();
    assert!(!restored.active);
    assert_eq!(restored.run.unwrap().items.len(), 512);
    assert!(
        store
            .save_setup_payload(Some(&vec![0; 32 * 1024 * 1024 + 1]))
            .is_err()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn mapping_reuses_the_current_scope_and_delete_recreates_a_fresh_identity() {
    let store = SqliteRepository::in_memory().unwrap();
    let legacy = store.setup_account("legacy").unwrap();
    let fingerprint = "1".repeat(64);
    let scoped = store
        .setup_account_for_source("source", &fingerprint)
        .unwrap();
    assert_eq!(scoped.id, legacy.id);
    assert_eq!(
        store
            .setup_account_for_source("renamed", &fingerprint)
            .unwrap()
            .id,
        scoped.id
    );
    store.remove_account(&scoped.id).unwrap();
    let recreated = store
        .setup_account_for_source("source", &fingerprint)
        .unwrap();
    assert_ne!(recreated.id, scoped.id);
    assert_eq!(store.setup_account("legacy").unwrap().id, recreated.id);
}

#[test]
fn missing_per_database_key_does_not_block_independently_verified_databases() {
    let (root, mut request) = fixture();
    request.databases = plan_files(
        &root,
        &["first.db", "missing-key.db", "incomplete-key.db", "last.db"],
    );
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(ScenarioProvider::new()),
    );
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    let run = settle(&app, &run.id);
    assert_eq!(run.phase, "partial");
    assert!(run.items[0].source.is_some());
    assert!(run.items[3].source.is_some());
    for (index, code) in [(1, "KEY_NOT_FOUND"), (2, "KEY_SCAN_INCOMPLETE")] {
        assert!(run.items[index].source.is_none());
        assert!(!run.items[index].key_ready);
        assert!(run.items[index].skipped);
        assert_eq!(run.items[index].outcome.as_deref(), Some("key_deferred"));
        assert_eq!(run.items[index].error.as_deref(), Some(code));
    }
    assert_eq!(
        app.history_sources_local(&run.binding.account_id)
            .unwrap()
            .len(),
        2
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn authentication_failure_stops_before_later_imports() {
    let (root, mut request) = fixture();
    request.databases = plan_files(&root, &["first.db", "authfailed.db", "last.db"]);
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(ScenarioProvider::new()),
    );
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    let run = settle(&app, &run.id);
    assert_eq!(run.phase, "failed");
    assert_eq!(
        run.items[1].error.as_deref(),
        Some("KEY_OR_PAGE_AUTHENTICATION_FAILED")
    );
    assert_eq!(run.items[1].outcome.as_deref(), Some("failed"));
    assert!(run.items[2].source.is_none());
    assert!(!run.items[1].skipped);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn all_unsupported_is_partial_without_any_successful_source() {
    let (root, mut request) = fixture();
    request.databases = plan_files(&root, &["unsupported.db"]);
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(ScenarioProvider::new()),
    );
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    let run = settle(&app, &run.id);
    assert_eq!(run.phase, "partial");
    assert!(run.items.iter().all(|i| i.skipped && i.source.is_none()));
    assert!(
        app.history_sources_local(&run.binding.account_id)
            .unwrap()
            .is_empty()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn db_storage_categories_share_a_scope_but_neighbor_account_roots_do_not() {
    let (root, mut request) = fixture();
    request.databases = plan_files(
        &root,
        &[
            "first_account/db_storage/message/message_0.db",
            "first_account/db_storage/contact/contact.db",
            "second_account/db_storage/message/message_0.db",
        ],
    );
    let app = Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        Arc::new(ScenarioProvider::new()),
    );
    assert_eq!(
        app.setup_start_local(request.clone(), true)
            .err()
            .unwrap()
            .code,
        "SOURCE_ROOT_MISMATCH"
    );
    assert!(app.accounts_local().unwrap().is_empty());
    request.databases.pop();
    assert_eq!(
        app.setup_start_local(request, true)
            .unwrap()
            .run
            .unwrap()
            .items
            .len(),
        2
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn thirty_three_source_plan_survives_reload_and_external_key_revision_invalidates_resume() {
    let (root, mut request) = fixture();
    let names: Vec<String> = (0..33).map(|i| format!("source_{i}.db")).collect();
    request.databases = plan_files(&root, &names.iter().map(String::as_str).collect::<Vec<_>>());
    let store = Arc::new(SqliteRepository::in_memory().unwrap());
    let app = Application::new(store.clone(), Arc::new(ScenarioProvider::new()));
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    assert_eq!(run.items.len(), 33);
    app.setup_step_local(&run.id).unwrap();
    drop(app);
    let app = Application::new(store.clone(), Arc::new(ScenarioProvider::new()));
    assert!(!app.setup_status_local().unwrap().active);
    store.remove_key(&run.binding.account_id).unwrap();
    assert_eq!(
        app.setup_resume_local(&run.id, true, false)
            .err()
            .unwrap()
            .code,
        "CONNECTION_CHANGED"
    );
    assert!(app.setup_status_local().unwrap().run.unwrap().items[0].key_ready);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_setup_item_without_outcome_deserializes() {
    let item: werc_domain::onboarding::SetupItem = serde_json::from_value(serde_json::json!({
        "path": "old.db", "key_ready": false, "skipped": false,
        "source": null, "error": null
    }))
    .unwrap();
    assert!(item.outcome.is_none());
}

#[test]
fn setup_rejects_duplicate_paths_and_excess_sources_before_registration() {
    let (root, mut request) = fixture();
    let store = Arc::new(SqliteRepository::in_memory().unwrap());
    let app = Application::new(
        store,
        Arc::new(Provider {
            gate: None,
            ready: AtomicBool::new(true),
            fail: AtomicBool::new(false),
        }),
    );
    request.databases.push(request.databases[0].clone());
    assert!(app.setup_start_local(request.clone(), true).is_err());
    request.databases = vec![request.databases[0].clone(); 513];
    assert!(app.setup_start_local(request, true).is_err());
    assert!(app.accounts_local().unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamPauseAt {
    AfterFirstBatch,
    AfterEndBeforeReturn,
}

struct PausableStreamProvider {
    base: ScenarioProvider,
    pause_at: StreamPauseAt,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
    armed: AtomicBool,
    attempts: std::sync::Mutex<Vec<String>>,
}
impl PausableStreamProvider {
    fn pause_point(&self, path: &str, point: StreamPauseAt) {
        if self.pause_at == point
            && std::path::Path::new(path).file_name().unwrap() == "second.db"
            && self.armed.swap(false, Ordering::SeqCst)
        {
            self.entered.wait();
            self.release.wait();
        }
    }
}
impl CapabilityProvider for PausableStreamProvider {
    fn capabilities(&self) -> Vec<Capability> {
        self.base.capabilities()
    }
    fn inspect(&self, path: &str) -> AppResult<ClientInstallation> {
        self.base.inspect(path)
    }
    fn offline_ready(&self, path: &str) -> AppResult<bool> {
        self.base.offline_ready(path)
    }
    fn acquire_database_key(
        &self,
        i: &ClientInstallation,
        pid: u32,
        path: &str,
    ) -> AppResult<AcquiredDatabaseKey> {
        self.base.acquire_database_key(i, pid, path)
    }
    fn database_salt(&self, path: &str) -> AppResult<[u8; 16]> {
        self.base.database_salt(path)
    }
    fn protect_key(&self, c: &str, b: &[u8]) -> AppResult<Vec<u8>> {
        self.base.protect_key(c, b)
    }
    fn unprotect_key(&self, c: &str, b: &[u8]) -> AppResult<Vec<u8>> {
        self.base.unprotect_key(c, b)
    }
    fn stream_database(
        &self,
        request: &HistoryReadRequest,
        visitor: &mut dyn FnMut(werc_domain::history::HistoryReadEvent) -> AppResult<()>,
    ) -> AppResult<()> {
        use sha2::{Digest, Sha256};
        use werc_domain::history::{HistoryMessage, HistoryReadEvent};
        let name = std::path::Path::new(&request.path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        self.attempts.lock().unwrap().push(name.into());
        let template = self.base.read_database(request)?;
        visitor(HistoryReadEvent::Begin {
            schema: template.schema.clone(),
            source_sha256: template.source_sha256.clone(),
            encryption: template.encryption.clone(),
            wal_committed_frames: 0,
        })?;
        for sequence in 0..2u64 {
            let mut data = template.clone();
            data.messages.push(HistoryMessage {
                id: "reader-placeholder".into(),
                conversation_id: name.into(),
                sender_id: Some("synthetic-sender".into()),
                local_id: sequence as i64 + 1,
                server_id: (sequence + 1).to_string(),
                timestamp: 1_700_000_000 + sequence as i64,
                sequence: sequence as i64,
                message_type: 1,
                text: Some(format!("synthetic {name} message {sequence}")),
                content_status: "decoded".into(),
            });
            let digest = hex::encode(Sha256::digest(serde_json::to_vec(&data).unwrap()));
            visitor(HistoryReadEvent::Batch {
                sequence,
                digest,
                data,
            })?;
            if sequence == 0 {
                self.pause_point(&request.path, StreamPauseAt::AfterFirstBatch);
            }
        }
        visitor(HistoryReadEvent::End {
            batches: 2,
            message_count: 2,
            contact_count: 0,
            session_count: 0,
        })?;
        self.pause_point(&request.path, StreamPauseAt::AfterEndBeforeReturn);
        Ok(())
    }
}

fn paused_stream_query() -> werc_domain::history::HistoryQuery {
    werc_domain::history::HistoryQuery {
        conversation_id: None,
        sender_id: None,
        message_type: None,
        search: None,
        from: None,
        until: None,
        cursor: None,
        limit: 10,
    }
}

fn assert_stream_pause_rolls_back_only_the_unfinished_source(pause_at: StreamPauseAt) {
    let (root, mut request) = fixture();
    request.databases = plan_files(&root, &["first.db", "second.db"]);
    let store = Arc::new(SqliteRepository::open(&root.join("state.db")).unwrap());
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let provider = Arc::new(PausableStreamProvider {
        base: ScenarioProvider::new(),
        pause_at,
        entered: entered.clone(),
        release: release.clone(),
        armed: AtomicBool::new(true),
        attempts: std::sync::Mutex::new(Vec::new()),
    });
    let app = Arc::new(Application::new(store.clone(), provider.clone()));
    let run = app.setup_start_local(request, true).unwrap().run.unwrap();
    // Acquire both keys, then fully commit the first source before pausing the
    // next source. This distinguishes per-source rollback from a whole-run reset.
    app.setup_step_local(&run.id).unwrap();
    app.setup_step_local(&run.id).unwrap();
    let first_run = app.setup_step_local(&run.id).unwrap().run.unwrap();
    let first_source = first_run.items[0].source.as_ref().unwrap().id.clone();
    assert_eq!(first_run.items[0].outcome.as_deref(), Some("imported"));
    assert_eq!(
        app.history_messages_local(&run.binding.account_id, &paused_stream_query())
            .unwrap()
            .data
            .len(),
        2
    );
    let revision = store.key_revision(&run.binding.account_id).unwrap();
    let id = run.id.clone();
    let worker = app.clone();
    let task = std::thread::spawn(move || worker.setup_step_local(&id));
    entered.wait();
    let working_status = app.setup_status_local();
    let visible_during_stream = app.history_sources_local(&run.binding.account_id);
    let messages_during_stream =
        app.history_messages_local(&run.binding.account_id, &paused_stream_query());
    let connection = rusqlite::Connection::open(root.join("state.db")).unwrap();
    let staged_during_stream: i64 = connection
        .query_row(
            "SELECT count(*) FROM history_sources WHERE visible=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let staged_batches: i64 = connection.query_row(
        "SELECT count(*) FROM history_batches b JOIN history_sources s ON s.id=b.source_id WHERE s.visible=0",
        [], |r| r.get(0),
    ).unwrap();
    app.setup_pause_local().unwrap();
    let reset = app.setup_reset_local();
    release.wait();
    let paused = task.join().unwrap().unwrap();

    let working = working_status.unwrap();
    assert!(working.working);
    assert_eq!(working.run.unwrap().phase, "importing");
    let visible_during_stream = visible_during_stream.unwrap();
    assert_eq!(visible_during_stream.len(), 1);
    assert_eq!(visible_during_stream[0].id, first_source);
    assert_eq!(messages_during_stream.unwrap().data.len(), 2);
    assert_eq!(staged_during_stream, 1);
    assert_eq!(
        staged_batches,
        match pause_at {
            StreamPauseAt::AfterFirstBatch => 1,
            StreamPauseAt::AfterEndBeforeReturn => 2,
        }
    );
    assert_eq!(reset.err().unwrap().code, "BUSY");
    assert!(!paused.active);
    assert!(!paused.working);
    let paused_run = paused.run.unwrap();
    assert_eq!(paused_run.phase, "waiting");
    assert_eq!(
        paused_run.items[0].source.as_ref().unwrap().id,
        first_source
    );
    assert!(paused_run.items[1].source.is_none());
    assert!(paused_run.items[1].key_ready);
    assert!(!paused_run.items[1].skipped);
    assert_eq!(paused_run.items[1].outcome.as_deref(), Some("pending"));
    assert!(paused_run.items.iter().all(|i| i.error.is_none()));
    assert_eq!(
        store.key_revision(&run.binding.account_id).unwrap(),
        revision
    );
    assert!(
        store
            .protected_key(&run.binding.account_id)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .history_sources(&run.binding.account_id)
            .unwrap()
            .len(),
        1
    );
    for (table, expected) in [
        ("history_sources", 1),
        ("history_batches", 2),
        ("history_record_digests", 2),
        ("history_pending_events", 0),
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, expected, "{pause_at:?}: {table}");
    }
    let observed_events: i64 = connection
        .query_row(
            "SELECT count(*) FROM events WHERE event_type='history.message_observed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(observed_events, 2);
    assert_eq!(
        app.history_messages_local(&run.binding.account_id, &paused_stream_query())
            .unwrap()
            .data
            .len(),
        2
    );
    assert_eq!(
        app.setup_step_local(&run.id).err().unwrap().code,
        "CONSENT_REQUIRED"
    );
    assert_eq!(
        app.setup_resume_local(&run.id, false, false)
            .err()
            .unwrap()
            .code,
        "CONSENT_REQUIRED"
    );
    assert!(!app.setup_status_local().unwrap().active);
    let resumed = app.setup_resume_local(&run.id, true, false).unwrap();
    assert!(resumed.active);
    let completed = settle(&app, &run.id);
    assert_eq!(completed.phase, "complete");
    assert_eq!(completed.items[0].source.as_ref().unwrap().id, first_source);
    assert_eq!(completed.items[1].outcome.as_deref(), Some("imported"));
    assert!(
        completed
            .items
            .iter()
            .all(|i| i.source.is_some() && !i.skipped && i.error.is_none())
    );
    assert_eq!(
        store
            .history_sources(&run.binding.account_id)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        app.history_messages_local(&run.binding.account_id, &paused_stream_query())
            .unwrap()
            .data
            .len(),
        4
    );
    assert_eq!(
        *provider.attempts.lock().unwrap(),
        vec!["first.db", "second.db", "second.db"]
    );
    let committed_imports: i64 = connection
        .query_row(
            "SELECT count(*) FROM events WHERE event_type='history.imported'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(committed_imports, 2);
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM history_sources WHERE visible=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM history_pending_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(connection);
    drop(app);
    drop(store);
    drop(provider);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pause_after_a_streamed_batch_aborts_only_current_source_and_requires_renewed_consent() {
    assert_stream_pause_rolls_back_only_the_unfinished_source(StreamPauseAt::AfterFirstBatch);
}

#[test]
fn pause_after_end_before_provider_return_still_prevents_commit_and_can_resume() {
    assert_stream_pause_rolls_back_only_the_unfinished_source(StreamPauseAt::AfterEndBeforeReturn);
}

#[test]
fn repository_final_commit_transaction_rejects_a_stale_key_revision() {
    use werc_domain::history::{
        HistoryChange, HistoryRecordDigest, HistorySource, ProtectedHistory, ProtectedHistoryBatch,
    };
    let store = SqliteRepository::in_memory().unwrap();
    let account = store.register_account("synthetic transaction CAS").unwrap();
    let revision = store.key_revision(&account.id).unwrap();
    let source = HistorySource {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account.id.clone(),
        source_sha256: "3".repeat(64),
        schema: "synthetic CAS".into(),
        encryption: "synthetic".into(),
        message_count: 1,
        contact_count: 0,
        session_count: 0,
        imported_at: "2026-11-09T00:00:00Z".into(),
        real_wechat_verified: false,
    };
    // Repository-only CAS test: opaque sentinel batch bytes must NEVER be
    // published. Real AES-GCM/DPAPI round trips are covered by the stream tests.
    let protected = ProtectedHistory {
        version: 2,
        changes: Vec::new(),
        source: source.clone(),
        wrapped_key: WindowsWechatProvider
            .protect_key("synthetic CAS source", &[0x77; 32])
            .unwrap(),
        nonce: vec![0; 12],
        ciphertext: vec![0x88; 16],
    };
    store.begin_history(&protected).unwrap();
    store
        .append_history_batch(
            &account.id,
            &source.id,
            &ProtectedHistoryBatch {
                sequence: 0,
                nonce: vec![0; 12],
                ciphertext: vec![0x99; 16],
                records: vec![HistoryRecordDigest {
                    message_id: "4".repeat(64),
                    digest: "5".repeat(64),
                }],
                changes: vec![HistoryChange {
                    message_id: "4".repeat(64),
                    change: "added",
                }],
            },
        )
        .unwrap();
    // This happens after the caller's last revision check, simulating the
    // check-to-commit race which only the final IMMEDIATE transaction can close.
    store.remove_key(&account.id).unwrap();
    assert_ne!(store.key_revision(&account.id).unwrap(), revision);
    assert_eq!(
        store
            .finish_history(&protected, revision)
            .err()
            .unwrap()
            .code,
        "CONNECTION_CHANGED"
    );
    assert!(store.history_sources(&account.id).unwrap().is_empty());
    assert!(store.history_payload(&account.id, &source.id).is_err());
    assert!(
        store
            .history_batch(&account.id, &source.id, 0)
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .events(0, &account.id, 100)
            .unwrap()
            .iter()
            .all(|e| e.event_type != "history.imported"
                && e.event_type != "history.message_observed")
    );
    // As with the application import path, the caller then removes its hidden
    // staging rows; rejecting a stale finish must not emit committed events.
    store.abort_history(&account.id, &source.id).unwrap();
    assert!(store.history_sources(&account.id).unwrap().is_empty());
}
