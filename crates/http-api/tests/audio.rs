mod common;
use std::sync::Arc;
use werc_application::{Application, ports::Repository};
use werc_domain::operation::OperationState;
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

fn wav() -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend(40u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(32000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(4u32.to_le_bytes());
    b.extend([0x3a, 0x19, 0x52, 0x12]);
    b
}
#[test]
#[cfg(windows)]
fn audio_is_encrypted_scoped_and_locally_processed_with_persistent_idempotency() {
    let (repo, a, b) = common::legacy_pair("音频合成账号", "隔离合成账号");
    let app = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
    let token = app
        .issue_token_local(
            "音频合成",
            vec![a.id.clone()],
            vec![
                "media:read".into(),
                "media:write".into(),
                "operations:create".into(),
                "operations:read".into(),
            ],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let p = app.authenticate(&token.secret).unwrap();
    let asset = app.upload_audio(&p, &a.id, wav()).unwrap();
    assert_eq!(asset.audio.frames, 2);
    assert_ne!(repo.asset(&a.id, &asset.id).unwrap().ciphertext, wav());
    assert_eq!(*app.download_audio(&p, &a.id, &asset.id).unwrap(), wav());
    assert_eq!(*app.audio_preview_local(&a.id, &asset.id).unwrap(), wav());
    assert!(app.audio_preview_local(&b.id, &asset.id).is_err());
    assert!(app.download_audio(&p, &b.id, &asset.id).is_err());
    assert!(app.upload_audio(&p, &b.id, wav()).is_err());
    assert!(app.upload_audio(&p, &a.id, b"not audio".to_vec()).is_err());
    let operation = app
        .inspect_audio_task(&p, &a.id, &asset.id, "synthetic-audio-operation-1")
        .unwrap();
    assert_eq!(operation.state, OperationState::Queued);
    assert_eq!(
        operation.id,
        app.inspect_audio_task(&p, &a.id, &asset.id, "synthetic-audio-operation-1")
            .unwrap()
            .id
    );
    assert!(app.run_local_task_once().unwrap());
    let finished = app.operation(&p, &a.id, &operation.id).unwrap();
    assert_eq!(finished.state, OperationState::Succeeded);
    assert_eq!(finished.result.unwrap()["sent"], false);
    assert!(!app.run_local_task_once().unwrap());
    let other_asset = app.upload_audio(&p, &a.id, wav()).unwrap();
    assert_eq!(
        app.inspect_audio_task(&p, &a.id, &other_asset.id, "synthetic-audio-operation-1")
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    let canceled = app
        .inspect_audio_task(&p, &a.id, &asset.id, "synthetic-audio-operation-2")
        .unwrap();
    app.cancel_operation(&p, &a.id, &canceled.id).unwrap();
    assert!(!app.run_local_task_once().unwrap());
    let revoked = app
        .inspect_audio_task(&p, &a.id, &asset.id, "synthetic-audio-operation-3")
        .unwrap();
    app.revoke_token_local(&token.info.id).unwrap();
    assert!(!app.run_local_task_once().unwrap());
    assert_eq!(
        repo.operation(&a.id, &revoked.id)
            .unwrap()
            .error_code
            .as_deref(),
        Some("AUTH_REVOKED")
    );
    app.delete_audio_local(&a.id, &asset.id).unwrap();
    assert!(repo.asset(&a.id, &asset.id).is_err());
}
#[test]
#[cfg(windows)]
fn revocation_during_preparation_discards_result_and_cancellation_wins() {
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let app = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
    let account = app.register_account_local("并发合成账号").unwrap().id;
    let asset = app.upload_audio_local(&account, wav()).unwrap();
    let token = app
        .issue_token_local(
            "本地任务",
            vec![account.clone()],
            vec!["operations:create".into(), "media:read".into()],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let p = app.authenticate(&token.secret).unwrap();
    let queued = app
        .inspect_audio_task(&p, &account, &asset.id, "synthetic-revoke-preparing")
        .unwrap();
    let mut running = repo.claim_operation().unwrap().unwrap();
    assert_eq!(running.id, queued.id);
    app.revoke_token_local(&token.info.id).unwrap();
    running.state = OperationState::Succeeded;
    running.result = Some(serde_json::json!({"sent":false,"marker":"discarded"}));
    repo.finish_operation(&running).unwrap();
    let done = repo.operation(&account, &queued.id).unwrap();
    assert_eq!(done.state, OperationState::Canceled);
    assert_eq!(done.error_code.as_deref(), Some("AUTH_REVOKED"));
    assert!(done.result.is_none());
    let queued = app
        .inspect_audio_task_local(&account, &asset.id, "synthetic-cancel-preparing")
        .unwrap();
    let mut running = repo.claim_operation().unwrap().unwrap();
    app.cancel_operation_local(&account, &queued.id).unwrap();
    running.state = OperationState::Succeeded;
    running.result = Some(serde_json::json!({"sent":false}));
    repo.finish_operation(&running).unwrap();
    assert_eq!(
        repo.operation(&account, &queued.id).unwrap().state,
        OperationState::Canceled
    );
    assert!(
        repo.operation(&account, &queued.id)
            .unwrap()
            .result
            .is_none()
    );
}

#[test]
#[cfg(windows)]
fn ciphertext_tamper_fails_and_restart_never_replays_submitting() {
    let path = std::env::temp_dir().join(format!("werc-audio-{}.sqlite", uuid::Uuid::new_v4()));
    let (account, asset, operation);
    {
        let repo = Arc::new(SqliteRepository::open(&path).unwrap());
        let app = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
        account = app.register_account_local("重启合成").unwrap().id;
        asset = app.upload_audio_local(&account, wav()).unwrap().id;
        operation = app
            .inspect_audio_task_local(&account, &asset, "synthetic-restart-key")
            .unwrap()
            .id;
        assert!(repo.claim_operation().unwrap().is_some());
    }
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute(
            "UPDATE audio_assets SET ciphertext=zeroblob(length(ciphertext))",
            [],
        )
        .unwrap();
    }
    {
        let repo = Arc::new(SqliteRepository::open(&path).unwrap());
        let app = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
        assert_eq!(app.recover_local_tasks().unwrap(), 1);
        assert_eq!(
            repo.operation(&account, &operation).unwrap().state,
            OperationState::Failed
        );
        assert!(!app.run_local_task_once().unwrap());
        let token = app
            .issue_token_local(
                "篡改验证",
                vec![account.clone()],
                vec!["media:read".into()],
                1,
            )
            .unwrap();
        app.enable_api(true);
        let p = app.authenticate(&token.secret).unwrap();
        assert!(app.download_audio(&p, &account, &asset).is_err());
        let second = app
            .inspect_audio_task_local(&account, &asset, "synthetic-submitting-key")
            .unwrap();
        assert!(repo.claim_operation().unwrap().is_some());
        drop(app);
        drop(repo);
        let c = rusqlite::Connection::open(&path).unwrap();
        let mut second = second;
        second.state = OperationState::Submitting;
        c.execute(
            "UPDATE operations SET state='submitting',record=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&second).unwrap(), second.id],
        )
        .unwrap();
    }
    {
        let repo = Arc::new(SqliteRepository::open(&path).unwrap());
        let app = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
        assert_eq!(app.recover_local_tasks().unwrap(), 1);
        assert!(
            repo.operations(&account)
                .unwrap()
                .iter()
                .any(|o| o.state == OperationState::Unknown)
        );
        assert!(!app.run_local_task_once().unwrap());
    }
    std::fs::remove_file(path).unwrap();
}
