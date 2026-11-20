#![cfg(windows)]
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;
use werc_application::Application;
use werc_wechat_win::WindowsWechatProvider;

async fn request(
    router: axum::Router,
    method: &str,
    path: &str,
    secret: &str,
    bytes: Vec<u8>,
) -> (StatusCode, Vec<u8>) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {secret}"))
                .header("Content-Type", "multipart/form-data; boundary=owned-file")
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    if response.status() == StatusCode::OK && method == "GET" && path.contains("/files/") {
        assert_eq!(
            response.headers()["content-type"],
            "application/octet-stream"
        );
        assert_eq!(
            response.headers()["content-disposition"],
            "attachment; filename*=UTF-8''%E8%87%AA%E6%9C%89%E9%99%84%E4%BB%B6%2E%74%78%74"
        );
    }
    (
        response.status(),
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}
fn multipart(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body=format!("--owned-file\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"自有附件.txt\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n--owned-file--\r\n");
    body
}
#[tokio::test]
async fn file_http_upload_preserves_bytes_scopes_downloads_and_deletes_without_send() {
    let (repo, a, b) = common::legacy_pair("文件HTTP自有", "隔离文件自有");
    let app = Arc::new(Application::new(repo, Arc::new(WindowsWechatProvider)));
    let token = app
        .issue_token_local(
            "文件自有",
            vec![a.id.clone()],
            vec!["media:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let read = app
        .issue_token_local(
            "只读文件自有",
            vec![a.id.clone()],
            vec!["media:read".into()],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let png = b"owned attachment bytes\n";
    let uri = format!("/api/v1/media/files?account_id={}", a.id);
    assert_eq!(
        request(
            router.clone(),
            "POST",
            &uri,
            &read.secret,
            multipart("file", png)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            router.clone(),
            "POST",
            &uri,
            &token.secret,
            multipart("wrong", png)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for name in ["../escape.txt", "CON.txt", "bad:name.txt"] {
        let body = String::from_utf8(multipart("file", png))
            .unwrap()
            .replace("自有附件.txt", name)
            .into_bytes();
        assert_eq!(
            request(router.clone(), "POST", &uri, &token.secret, body)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let missing = String::from_utf8(multipart("file", png))
        .unwrap()
        .replace("; filename=\"自有附件.txt\"", "")
        .into_bytes();
    assert_eq!(
        request(router.clone(), "POST", &uri, &token.secret, missing)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            router.clone(),
            "POST",
            &uri,
            &token.secret,
            multipart("file", b"")
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let (status, body) = request(
        router.clone(),
        "POST",
        &uri,
        &token.secret,
        multipart("file", png),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let reply: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["sent"], false);
    let id = reply["data"]["id"].as_str().unwrap();
    assert_eq!(reply["data"]["size"], png.len());
    assert!(reply["data"].get("file_name").is_none());
    let download = format!("/api/v1/media/files/{id}?account_id={}", a.id);
    let (status, bytes) = request(router.clone(), "GET", &download, &read.secret, vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, png);
    let (_, body) = request(router.clone(), "GET", &uri, &read.secret, vec![]).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let other = format!("/api/v1/media/files/{id}?account_id={}", b.id);
    assert_eq!(
        request(router.clone(), "GET", &other, &token.secret, vec![])
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(router.clone(), "DELETE", &download, &read.secret, vec![])
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(router.clone(), "DELETE", &download, &token.secret, vec![])
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(router.clone(), "GET", &download, &token.secret, vec![])
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert!(app.sends_local(&a.id).unwrap().is_empty());
    app.revoke_token_local(&token.info.id).unwrap();
    assert_eq!(
        request(router, "POST", &uri, &token.secret, multipart("file", png))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}

struct DownloadGate {
    state: std::sync::Mutex<(bool, bool)>,
    wake: std::sync::Condvar,
}
impl werc_application::ports::CapabilityProvider for DownloadGate {
    fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        vec![]
    }
    fn protect_key(&self, c: &str, k: &[u8]) -> werc_domain::AppResult<Vec<u8>> {
        werc_application::ports::CapabilityProvider::protect_key(&WindowsWechatProvider, c, k)
    }
    fn unprotect_key(&self, c: &str, k: &[u8]) -> werc_domain::AppResult<Vec<u8>> {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.wake.notify_all();
        let (state, timeout) = self
            .wake
            .wait_timeout_while(state, std::time::Duration::from_secs(10), |s| !s.1)
            .unwrap();
        if timeout.timed_out() {
            return Err(werc_domain::AppError::internal());
        }
        drop(state);
        werc_application::ports::CapabilityProvider::unprotect_key(&WindowsWechatProvider, c, k)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_file_download_does_not_release_bytes_or_filename_after_decryption() {
    let (repo, a, _) = common::legacy_pair("自有文件撤权", "自有隔离");
    let gate = Arc::new(DownloadGate {
        state: std::sync::Mutex::new((false, false)),
        wake: std::sync::Condvar::new(),
    });
    let app = Arc::new(Application::new(repo, gate.clone()));
    let asset = app
        .upload_file_local(&a.id, "private-owned.txt", b"private owned bytes".to_vec())
        .unwrap();
    let token = app
        .issue_token_local("自有下载", vec![a.id.clone()], vec!["media:read".into()], 1)
        .unwrap();
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let uri = format!("/api/v1/media/files/{}?account_id={}", asset.id, a.id);
    let secret = token.secret.clone();
    let pending = tokio::spawn(async move {
        router
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("Authorization", format!("Bearer {secret}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    let observed = gate.clone();
    let entered = tokio::task::spawn_blocking(move || {
        let (state, timeout) = observed
            .wake
            .wait_timeout_while(
                observed.state.lock().unwrap(),
                std::time::Duration::from_secs(5),
                |s| !s.0,
            )
            .unwrap();
        !timeout.timed_out() && state.0
    })
    .await
    .unwrap();
    app.revoke_token_local(&token.info.id).unwrap();
    {
        let mut state = gate.state.lock().unwrap();
        state.1 = true;
        gate.wake.notify_all();
    }
    assert!(entered, "下载未进入解密边界");
    let response = pending.await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get("content-disposition").is_none());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    assert!(!body.contains("private-owned.txt") && !body.contains("private owned bytes"));
    assert!(app.sends_local(&a.id).unwrap().is_empty());
}
