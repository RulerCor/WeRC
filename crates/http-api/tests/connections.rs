mod common;
use std::sync::Arc;
use werc_application::{
    Application,
    ports::{CapabilityProvider, Repository},
};
use werc_domain::{AppResult, capability::Capability, connection::ClientInstallation};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

struct SyntheticProvider;
#[cfg(windows)]
struct UpgradeProvider(std::sync::atomic::AtomicBool);
#[cfg(windows)]
impl CapabilityProvider for UpgradeProvider {
    fn capabilities(&self) -> Vec<Capability> { vec![] }
    fn inspect(&self, path: &str) -> AppResult<ClientInstallation> {
        let mut i = SyntheticProvider.inspect(path)?;
        let upgraded = self.0.load(std::sync::atomic::Ordering::SeqCst);
        i.sha256 = if upgraded { "2" } else { "1" }.repeat(64);
        i.version = if upgraded { "4.2.0.53" } else { "4.2.0.52" }.into();
        i.compatibility = "native_check_required".into();
        Ok(i)
    }
    fn protect_key(&self, context: &str, key: &[u8]) -> AppResult<Vec<u8>> { WindowsWechatProvider.protect_key(context,key) }
    fn unprotect_key(&self, context: &str, key: &[u8]) -> AppResult<Vec<u8>> { WindowsWechatProvider.unprotect_key(context,key) }
    fn acquire_database_key(&self, i: &ClientInstallation, pid: u32, db: &str) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> { SyntheticProvider.acquire_database_key(i,pid,db) }
    fn database_salt(&self, db: &str) -> AppResult<[u8;16]> { SyntheticProvider.database_salt(db) }
}

#[test]
#[cfg(windows)]
fn same_path_upgrade_migrates_all_protected_keys_atomically() {
    use std::sync::atomic::Ordering;
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let provider = Arc::new(UpgradeProvider(std::sync::atomic::AtomicBool::new(false)));
    let app = Application::new(repo.clone(), provider.clone());
    let id = app.register_account_local("upgrade fixture").unwrap().id;
    let old = app.bind_client_local(&id, "fixture.exe").unwrap();
    app.acquire_database_key_local(&id, 123, "first", true).unwrap();
    app.acquire_database_key_local(&id, 123, "second", true).unwrap();
    let old_cipher = repo.protected_key(&id).unwrap().unwrap().0;
    let stale_revision = repo.key_revision(&id).unwrap();
    provider.0.store(true, Ordering::SeqCst);
    let expected = provider.inspect("fixture.exe").unwrap();
    let mut invalid = expected.clone(); invalid.compatibility = "adapter_pending".into();
    assert_eq!(app.refresh_upgraded_binding_local(&id,&invalid).unwrap_err().code, "NATIVE_BUILD_UNSUPPORTED");
    assert_eq!(repo.protected_key(&id).unwrap().unwrap().0,old_cipher);
    let new = app.refresh_upgraded_binding_local(&id,&expected).unwrap();
    assert_eq!(new.installation.sha256, "2".repeat(64));
    assert_eq!(app.key_status_local(&id).unwrap().verified_databases,2);
    let keys = app.reveal_database_keys_local(&id).unwrap();
    assert_eq!(keys.len(),2);
    assert!(keys.iter().all(|k|k.key_hex == "5a".repeat(32)));
    let new_cipher = repo.protected_key(&id).unwrap().unwrap().0;
    assert_ne!(old_cipher,new_cipher);
    assert_eq!(repo.refresh_binding_preserving_key(&old,&new,Some(&old_cipher),Some(&new_cipher),stale_revision).unwrap_err().code,"CONNECTION_CHANGED");
    assert_eq!(repo.protected_key(&id).unwrap().unwrap().0,new_cipher);
    assert_eq!(app.bindings_local().unwrap()[0].installation.sha256,new.installation.sha256);
}
impl CapabilityProvider for SyntheticProvider {
    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
    fn inspect(&self, path: &str) -> AppResult<ClientInstallation> {
        Ok(ClientInstallation {
            id: path.into(),
            executable_path: path.into(),
            version: "0.0.0.1".into(),
            architecture: "x86_64".into(),
            product_name: "synthetic".into(),
            sha256: "1".repeat(64),
            process_ids: vec![],
            sources: vec!["fixture".into()],
            compatibility: "unsupported".into(),
            reason: "合成元数据，不是微信登录".into(),
        })
    }
    fn acquire_database_key(
        &self,
        _: &ClientInstallation,
        _: u32,
        database: &str,
    ) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
        if database == "bad" {
            return Err(werc_domain::AppError::new("KEY_NOT_FOUND", "合成失败"));
        }
        let salt = if database == "second" { 2 } else { 1 };
        Ok(werc_domain::connection::AcquiredDatabaseKey {
            key: zeroize::Zeroizing::new([0x5a; 32]),
            salt: [salt; 16],
            first_page_sha256: [3; 32],
        })
    }
    fn database_salt(&self, database: &str) -> AppResult<[u8; 16]> {
        Ok([match database {
            "first" => 1,
            "second" => 2,
            _ => 3,
        }; 16])
    }
    fn acquire_database_key_controlled(
        &self, installation: &ClientInstallation, pid: u32, database: &str,
        control: &werc_application::ports::KeyScanControl<'_>,
    ) -> AppResult<werc_domain::connection::AcquiredDatabaseKey> {
        control.check()?;
        (control.progress)(4096);
        self.acquire_database_key(installation, pid, database)
    }
    fn read_database(
        &self,
        request: &werc_domain::history::HistoryReadRequest,
    ) -> AppResult<werc_domain::history::HistoryBundle> {
        assert_eq!(request.key_hex.as_deref(), Some("5a".repeat(32).as_str()));
        Ok(werc_domain::history::HistoryBundle {
            schema: "synthetic".into(),
            source_sha256: "3".repeat(64),
            encryption: "synthetic".into(),
            wal_committed_frames: 0,
            messages: vec![],
            contacts: vec![],
            sessions: vec![],
            warnings: vec![],
        })
    }
    fn protect_key(&self, account: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.protect_key(account, key)
    }
    fn unprotect_key(&self, account: &str, key: &[u8]) -> AppResult<Vec<u8>> {
        WindowsWechatProvider.unprotect_key(account, key)
    }
}

#[test]
#[cfg(windows)]
fn canceled_acquisition_never_commits_candidate_and_keeps_previous_verified_key() {
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let app = Application::new(repo.clone(), Arc::new(SyntheticProvider));
    let account = app.register_account_local("自动化测试：取消取钥").unwrap();
    app.bind_client_local(&account.id, "synthetic.exe").unwrap();
    app.acquire_database_key_local(&account.id, 123, "first", true).unwrap();
    let before = repo.protected_key(&account.id).unwrap().unwrap();
    let request = "owned-key-cancel-during-scan";
    let result = app.acquire_database_key_cancellable_local(&account.id, 123, "second", true, request, 30, &|bytes| {
        assert_eq!(bytes, 4096);
        app.cancel_browse_local(request).unwrap();
    });
    assert_eq!(result.unwrap_err().code, "QUERY_CANCELED");
    assert_eq!(repo.protected_key(&account.id).unwrap().unwrap(), before);
    assert_eq!(app.key_status_local(&account.id).unwrap().verified_databases, 1);
    let pre = "owned-key-cancel-before-start";
    app.cancel_browse_local(pre).unwrap();
    assert_eq!(app.acquire_database_key_cancellable_local(&account.id, 123, "second", true, pre, 30, &|_| panic!("pre-canceled scan ran")).unwrap_err().code, "QUERY_CANCELED");
    assert_eq!(app.acquire_database_key_cancellable_local(&account.id, 123, "second", true, "owned-key-invalid-timeout", 31, &|_| {}).unwrap_err().code, "INVALID_ARGUMENT");
}

#[test]
fn binding_reopens_preserves_registration_and_scopes() {
    let path = std::env::temp_dir().join(format!("werc-binding-{}.sqlite", uuid::Uuid::new_v4()));
    let id;
    {
        let app = Application::new(
            Arc::new(SqliteRepository::open(&path).unwrap()),
            Arc::new(SyntheticProvider),
        );
        id = app.register_account_local("合成来源").unwrap().id;
        let binding = app.bind_client_local(&id, "synthetic.exe").unwrap();
        assert_eq!(binding.validation, "metadata_only");
        assert_eq!(
            app.accounts_local().unwrap()[0].state,
            werc_domain::AccountState::Unsupported
        );
        assert!(!app.status().real_wechat_verified);
    }
    {
        let app = Application::new(
            Arc::new(SqliteRepository::open(&path).unwrap()),
            Arc::new(SyntheticProvider),
        );
        assert_eq!(app.bindings_local().unwrap()[0].account_id, id);
        assert_eq!(
            app.register_account_local("另一个合成来源")
                .unwrap_err()
                .code,
            "SINGLE_CONNECTION_REQUIRED"
        );
        let token = app
            .issue_token_local(
                "合成读取",
                vec![id.clone()],
                vec!["connections:read".into()],
                7,
            )
            .unwrap();
        app.enable_api(true);
        let principal = app.authenticate(&token.secret).unwrap();
        assert!(app.binding(&principal, &id).unwrap().is_some());
        assert!(app.binding(&principal, "fixture-archive").is_err());
        app.unbind_client_local(&id).unwrap();
        assert!(app.binding(&principal, &id).unwrap().is_none());
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
#[cfg(windows)]
fn synthetic_import_is_protected_not_verified_and_cleared_on_change() {
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let app = Application::new(repo.clone(), Arc::new(SyntheticProvider));
    let account = app.register_account_local("DPAPI合成").unwrap();
    let raw = "5a".repeat(32);
    assert!(app.import_key_local(&account.id, raw.clone()).is_err());
    app.bind_client_local(&account.id, "fixture-one").unwrap();
    assert!(app.import_key_local(&account.id, "bad".into()).is_err());
    let status = app.import_key_local(&account.id, raw).unwrap();
    assert!(status.protected);
    assert!(!status.verified);
    assert_ne!(
        repo.protected_key(&account.id).unwrap().unwrap().0,
        vec![0x5a; 32]
    );
    app.bind_client_local(&account.id, "fixture-one").unwrap();
    assert!(app.key_status_local(&account.id).unwrap().protected);
    app.bind_client_local(&account.id, "fixture-two").unwrap();
    assert!(!app.key_status_local(&account.id).unwrap().protected);
    app.import_key_local(&account.id, "5a".repeat(32)).unwrap();
    app.remove_key_local(&account.id).unwrap();
    assert!(!app.key_status_local(&account.id).unwrap().protected);
}

#[test]
#[cfg(windows)]
fn synthetic_acquisition_requires_consent_and_isolates_database_salts() {
    let (repo, current, other) = common::legacy_pair("合成取钥", "另一归档");
    let app = Application::new(repo.clone(), Arc::new(SyntheticProvider));
    let id = current.id;
    assert_eq!(
        app.acquire_database_key_local(&id, 123, "first", false)
            .unwrap_err()
            .code,
        "CONSENT_REQUIRED"
    );
    assert!(
        app.acquire_database_key_local(&id, 123, "first", true)
            .is_err()
    );
    app.bind_client_local(&id, "synthetic.exe").unwrap();
    assert!(
        app.acquire_database_key_local(&id, 123, "bad", true)
            .is_err()
    );
    assert!(!app.key_status_local(&id).unwrap().protected);
    let status = app
        .acquire_database_key_local(&id, 123, "first", true)
        .unwrap();
    assert!(status.verified);
    assert_eq!(status.verified_databases, 1);
    assert_eq!(
        app.acquire_database_key_local(&id, 123, "second", true)
            .unwrap()
            .verified_databases,
        2
    );
    assert_eq!(
        app.acquire_database_key_local(&id, 123, "first", true)
            .unwrap()
            .verified_databases,
        2
    );
    let secret = repo.protected_key(&id).unwrap().unwrap().0;
    assert!(!secret.windows(32).any(|w| w == [0x5a; 32]));
    let request = |path: &str| werc_domain::history::HistoryReadRequest {
        path: path.into(),
        key_hex: None,
        allow_plaintext: false,
    };
    assert!(
        app.import_history_local(&id, request("first"), true)
            .is_ok()
    );
    assert_eq!(
        app.import_history_local(&id, request("unknown"), true)
            .unwrap_err()
            .code,
        "KEY_SOURCE_MISMATCH"
    );
    assert_eq!(
        app.register_account_local("另一账号").unwrap_err().code,
        "SINGLE_CONNECTION_REQUIRED"
    );
    assert!(!app.key_status_local(&other.id).unwrap().protected);
    assert!(!app.status().real_wechat_verified);
    app.remove_key_local(&id).unwrap();
    assert!(!app.key_status_local(&id).unwrap().verified);
}

#[test]
#[cfg(windows)]
fn key_clear_or_rebinding_rejects_late_result_and_corrupt_key_remains_clearable() {
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let app = Application::new(repo.clone(), Arc::new(SyntheticProvider));
    let id = app.register_account_local("竞态合成").unwrap().id;
    let binding = app.bind_client_local(&id, "synthetic.exe").unwrap();
    let revision = repo.key_revision(&id).unwrap();
    app.remove_key_local(&id).unwrap();
    assert_eq!(
        repo.store_acquired_key(&binding, &[1; 32], revision)
            .unwrap_err()
            .code,
        "KEY_STATE_CHANGED"
    );
    assert!(repo.protected_key(&id).unwrap().is_none());
    let revision = repo.key_revision(&id).unwrap();
    app.unbind_client_local(&id).unwrap();
    app.bind_client_local(&id, "synthetic.exe").unwrap();
    assert!(
        repo.store_acquired_key(&binding, &[1; 32], revision)
            .is_err()
    );
    let binding = app.bindings_local().unwrap().remove(0);
    repo.store_key(&binding, b"corrupt protected bytes")
        .unwrap();
    let status = app.key_status_local(&id).unwrap();
    assert!(status.protected);
    assert!(!status.verified);
    assert!(status.verification_error.is_some());
    app.remove_key_local(&id).unwrap();
    assert!(!app.key_status_local(&id).unwrap().protected);
}

#[test]
fn migrates_v1_without_resetting_account_or_token() {
    let path = std::env::temp_dir().join(format!("werc-migrate-{}.sqlite", uuid::Uuid::new_v4()));
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(include_str!("../../infrastructure/migrations/0001.sql"))
            .unwrap();
        c.execute(
            "INSERT INTO accounts(id,label,created_at) VALUES ('legacy','旧标签','2026-01-01')",
            [],
        )
        .unwrap();
    }
    {
        let repo = SqliteRepository::open(&path).unwrap();
        assert_eq!(repo.accounts().unwrap()[0].id, "legacy");
        assert!(repo.bindings().unwrap().is_empty());
        assert!(repo.protected_key("legacy").unwrap().is_none());
    }
    std::fs::remove_file(path).unwrap();
}
