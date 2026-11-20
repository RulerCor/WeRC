mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use werc_application::{Application, TokenValidity, ports::Repository, token_digest};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::WindowsWechatProvider;

fn setup() -> (Arc<Application>, String, String) {
    let (repo, a, b) = common::legacy_pair("测试账号 A", "测试账号 B");
    let app = Arc::new(Application::new(repo, Arc::new(WindowsWechatProvider)));
    let token = app
        .issue_token_local(
            "契约测试",
            vec![a.id.clone()],
            vec![
                "accounts:read".into(),
                "capabilities:read".into(),
                "events:read".into(),
                "messages:send".into(),
                "operations:create".into(),
                "moments:publish".into(),
            ],
            7,
        )
        .unwrap();
    app.enable_api(true);
    (app, token.secret, b.id)
}

#[tokio::test]
async fn configurable_token_expiration_and_revocation() {
    let (repo, account, _) = common::legacy_pair("期限测试", "其他账号");
    let app = Arc::new(Application::new(repo, Arc::new(WindowsWechatProvider)));
    let issue = |validity| {
        app.issue_token_with_validity_local(
            "期限测试",
            vec![account.id.clone()],
            vec!["accounts:read".into()],
            validity,
        )
    };
    for seconds in [0, -1, i64::MAX] {
        assert_eq!(issue(TokenValidity::Timed { seconds }).unwrap_err().code, "INVALID_ARGUMENT");
    }
    let permanent = issue(TokenValidity::Permanent).unwrap();
    let short = issue(TokenValidity::Timed { seconds: 2 }).unwrap();
    let longer = issue(TokenValidity::Timed { seconds: 91 * 86_400 }).unwrap();
    assert_ne!(longer.info.expires_at, permanent.info.expires_at);
    let created = chrono::DateTime::parse_from_rfc3339(&short.info.created_at).unwrap();
    let expires = chrono::DateTime::parse_from_rfc3339(&short.info.expires_at).unwrap();
    assert_eq!((expires - created).num_seconds(), 2);
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let status = |secret: &str| {
        let request = Request::builder()
            .uri("/api/v1/accounts")
            .header("Authorization", format!("Bearer {secret}"))
            .body(Body::empty())
            .unwrap();
        let router = router.clone();
        async move { router.oneshot(request).await.unwrap().status() }
    };
    assert_eq!(status(&short.secret).await, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert_eq!(status(&short.secret).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status(&permanent.secret).await, StatusCode::OK);
    app.revoke_token_local(&permanent.info.id).unwrap();
    assert_eq!(status(&permanent.secret).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn account_isolation_and_revocation() {
    let (app, secret, _) = setup();
    let router = werc_http_api::router(app.clone());
    let request = || {
        Request::builder()
            .uri("/api/v1/accounts")
            .header("Authorization", format!("Bearer {secret}"))
            .body(Body::empty())
            .unwrap()
    };
    let response = router.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"][0]["label"], "测试账号 A");
    let token = app.tokens_local().unwrap().remove(0);
    app.revoke_token_local(&token.id).unwrap();
    assert_eq!(
        router.oneshot(request()).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn rejects_missing_bearer_origin_and_paused_api() {
    let (app, secret, _) = setup();
    let router = werc_http_api::router(app.clone());
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/accounts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/accounts")
                .header("Origin", "https://untrusted.example")
                .header("Authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    app.enable_api(false);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/accounts")
                .header("Authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn unsupported_writes_never_report_success() {
    let (app, secret, other_account) = setup();
    let own = app
        .accounts_local()
        .unwrap()
        .into_iter()
        .find(|a| a.label == "测试账号 A")
        .unwrap();
    let router = werc_http_api::router(app.clone());
    let request = |id: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/messages/send-text")
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "test-0123456789abcdef")
            .body(Body::from(
                json!({"account_id":id,"username":"filehelper","text":"测试，未实际发送"})
                    .to_string(),
            ))
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(request(&other_account))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = router.oneshot(request(&own.id)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["error"]["code"], "CAPABILITY_NOT_IMPLEMENTED");
}

#[tokio::test]
async fn event_filter_cursor_and_sse_replay() {
    let (app, secret, other_account) = setup();
    let principal = app.authenticate(&secret).unwrap();
    let own = &principal.account_ids[0];
    let events = app.events(&principal, own, 0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "connection.selected");
    assert!(app.events(&principal, &other_account, 0, 100).is_err());
    assert!(
        app.events(&principal, own, events[0].event_id, 100)
            .unwrap()
            .is_empty()
    );
    let response = werc_http_api::router(app.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/events/stream?account_id={own}"))
                .header("Authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let data = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
    assert!(data.contains("connection.selected"));
    app.enable_api(false);
    let frame = tokio::time::timeout(std::time::Duration::from_secs(2), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        String::from_utf8(frame.into_data().unwrap().to_vec())
            .unwrap()
            .contains("stream.closed")
    );
}

#[test]
fn store_persists_hash_only_and_revokes_removed_account() {
    let path = std::env::temp_dir().join(format!("werc-test-{}.sqlite", uuid::Uuid::new_v4()));
    let account_id;
    {
        let repo = SqliteRepository::open(&path).unwrap();
        let account = repo.register_account("持久测试").unwrap();
        account_id = account.id;
        let token = werc_domain::TokenInfo {
            id: "test-token".into(),
            label: "持久测试".into(),
            account_ids: vec![account_id.clone()],
            scopes: vec!["accounts:read".into()],
            created_at: "2026-01-01T00:00:00+00:00".into(),
            expires_at: "2099-01-01T00:00:00+00:00".into(),
            revoked: false,
        };
        repo.create_token(&token, &token_digest("not-a-real-secret"))
            .unwrap();
        assert!(
            repo.authenticate(&token_digest("not-a-real-secret"))
                .is_ok()
        );
    }
    {
        let repo = SqliteRepository::open(&path).unwrap();
        assert_eq!(repo.accounts().unwrap().len(), 1);
        repo.remove_account(&account_id).unwrap();
        assert!(
            repo.authenticate(&token_digest("not-a-real-secret"))
                .is_err()
        );
    }
    assert!(!String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains("not-a-real-secret"));
    std::fs::remove_file(path).unwrap();
}
