mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
use werc_application::Application;
use werc_wechat_win::WindowsWechatProvider;

fn wave() -> Vec<u8> {
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
async fn request(
    router: axum::Router,
    method: &str,
    path: &str,
    secret: &str,
    mime: &str,
    body: Vec<u8>,
) -> (StatusCode, Vec<u8>) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {secret}"))
                .header("Content-Type", mime)
                .header("Idempotency-Key", "synthetic-contract-task-1")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
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
#[tokio::test]
#[cfg(windows)]
async fn multipart_audio_and_local_operation_are_not_native_voice_delivery() {
    let (repo, account, other) = common::legacy_pair("HTTP音频合成", "HTTP隔离合成");
    let app = Arc::new(Application::new(repo, Arc::new(WindowsWechatProvider)));
    let token = app
        .issue_token_local(
            "合成音频",
            vec![account.id.clone()],
            vec![
                "media:read".into(),
                "media:write".into(),
                "operations:create".into(),
                "operations:read".into(),
                "messages:send_voice".into(),
                "connections:read".into(),
            ],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let mut multipart = b"--synthetic-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"ignored-path.wav\"\r\nContent-Type: application/octet-stream\r\n\r\n".to_vec();
    multipart.extend(wave());
    multipart.extend(b"\r\n--synthetic-boundary--\r\n");
    let uri = format!("/api/v1/media/audio?account_id={}", account.id);
    let mime = "multipart/form-data; boundary=synthetic-boundary";
    let (status, body) = request(
        router.clone(),
        "POST",
        &uri,
        &token.secret,
        mime,
        multipart.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let reply: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["sent"], false);
    let asset = reply["data"]["id"].as_str().unwrap();
    let download = format!("/api/v1/media/audio/{asset}?account_id={}", account.id);
    let (status, body) = request(router.clone(), "GET", &download, &token.secret, "", vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, wave());
    let (status, _) = request(
        router.clone(),
        "POST",
        &format!("/api/v1/media/audio?account_id={}", other.id),
        &token.secret,
        mime,
        multipart,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = request(
        router.clone(),
        "GET",
        &format!("/api/v1/connections?account_id={}", other.id),
        &token.secret,
        "",
        vec![],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let voice =
        json!({"account_id":account.id,"username":"synthetic-recipient-not-real","asset_id":asset});
    let (status, _) = request(
        router.clone(),
        "POST",
        "/api/v1/messages/send-voice",
        &token.secret,
        "application/json",
        serde_json::to_vec(&voice).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(app.operations_local(&account.id).unwrap().is_empty());
    let task = serde_json::to_vec(&json!({"account_id":account.id,"asset_id":asset})).unwrap();
    let (status, body) = request(
        router.clone(),
        "POST",
        "/api/v1/operations",
        &token.secret,
        "application/json",
        task.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let result: Value = serde_json::from_slice(&body).unwrap();
    let (_, duplicate) = request(
        router.clone(),
        "POST",
        "/api/v1/operations",
        &token.secret,
        "application/json",
        task,
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&duplicate).unwrap()["data"]["id"],
        result["data"]["id"]
    );
    assert!(app.run_local_task_once().unwrap());
    let path = format!(
        "/api/v1/operations/{}?account_id={}",
        result["data"]["id"].as_str().unwrap(),
        account.id
    );
    let (_, body) = request(router.clone(), "GET", &path, &token.secret, "", vec![]).await;
    let completed: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(completed["data"]["state"], "succeeded");
    assert_eq!(completed["data"]["result"]["sent"], false);
    let (status, _) = request(router, "DELETE", &download, &token.secret, "", vec![]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}
