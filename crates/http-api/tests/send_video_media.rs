#![cfg(windows)]
mod common;
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;
use werc_application::Application;
use werc_wechat_win::WindowsWechatProvider;

fn multipart(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body=format!("--owned-video\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: video/mp4\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n--owned-video--\r\n");
    body
}
fn request(method: &str, uri: &str, token: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "multipart/form-data; boundary=owned-video")
        .header("Range", "bytes=0-10")
        .body(Body::from(body))
        .unwrap()
}
#[tokio::test]
async fn video_upload_scope_content_validation_binary_read_and_delete() {
    let (repo, a, b) = common::legacy_pair("自有视频HTTP", "隔离视频HTTP");
    let app = Arc::new(Application::new(repo, Arc::new(WindowsWechatProvider)));
    let issue = |scopes: Vec<String>| {
        app.issue_token_local("自有视频", vec![a.id.clone()], scopes, 1)
            .unwrap()
    };
    let all = issue(vec!["media:read".into(), "media:write".into()]);
    let read = issue(vec!["media:read".into()]);
    let write = issue(vec!["media:write".into()]);
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let bytes = include_bytes!("../../../tests/fixtures/owned-send-video.mp4");
    let uri = format!("/api/v1/media/videos?account_id={}", a.id);
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "POST",
                &uri,
                &read.secret,
                multipart("自动化测试.mp4", bytes)
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    for (name, content) in [
        ("wrong.txt", bytes.as_slice()),
        ("bad.mp4", b"not video".as_slice()),
        ("../escape.mp4", bytes.as_slice()),
    ] {
        assert!(
            !router
                .clone()
                .oneshot(request("POST", &uri, &all.secret, multipart(name, content)))
                .await
                .unwrap()
                .status()
                .is_success()
        );
    }
    let response = router
        .clone()
        .oneshot(request(
            "POST",
            &uri,
            &all.secret,
            multipart("自动化测试.mp4", bytes),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let json: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(json["sent"], false);
    let id = json["data"]["id"].as_str().unwrap();
    let uri = format!("/api/v1/media/videos/{id}?account_id={}", a.id);
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", &uri, &write.secret, vec![]))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request(
                "GET",
                &format!("/api/v1/media/videos/{id}?account_id={}", b.id),
                &all.secret,
                vec![]
            ))
            .await
            .unwrap()
            .status(),
        403
    );
    let response = router
        .clone()
        .oneshot(request("GET", &uri, &read.secret, vec![]))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "video/mp4");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["accept-ranges"], "none");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        &response.into_body().collect().await.unwrap().to_bytes()[..],
        bytes
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("DELETE", &uri, &read.secret, vec![]))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        router
            .clone()
            .oneshot(request("DELETE", &uri, &all.secret, vec![]))
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        router
            .oneshot(request("GET", &uri, &read.secret, vec![]))
            .await
            .unwrap()
            .status(),
        404
    );
    assert!(app.sends_local(&a.id).unwrap().is_empty());
}
