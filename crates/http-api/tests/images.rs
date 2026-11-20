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
                .header("Content-Type", "multipart/form-data; boundary=owned-image")
                .body(Body::from(bytes))
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
fn multipart(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body=format!("--owned-image\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"ignored.png\"\r\nContent-Type: image/png\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n--owned-image--\r\n");
    body
}
#[tokio::test]
async fn image_http_upload_normalizes_scopes_downloads_and_deletes_without_send() {
    let (repo, a, b) = common::legacy_pair("图片HTTP自有", "隔离图片自有");
    let app = Arc::new(Application::new(repo, Arc::new(WindowsWechatProvider)));
    let token = app
        .issue_token_local(
            "图片自有",
            vec![a.id.clone()],
            vec!["media:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let read = app
        .issue_token_local(
            "只读图片自有",
            vec![a.id.clone()],
            vec!["media:read".into()],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let png = include_bytes!("../../../tests/fixtures/owned-image.png");
    let uri = format!("/api/v1/media/images?account_id={}", a.id);
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
    assert_eq!(
        request(
            router.clone(),
            "POST",
            &uri,
            &token.secret,
            multipart("file", b"not-png")
        )
        .await
        .0,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
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
    assert_eq!(reply["data"]["image"]["width"], 2);
    let download = format!("/api/v1/media/images/{id}?account_id={}", a.id);
    let (status, bytes) = request(router.clone(), "GET", &download, &read.secret, vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, *werc_domain::image::canonical_png(png).unwrap().1);
    let (_, body) = request(router.clone(), "GET", &uri, &read.secret, vec![]).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let other = format!("/api/v1/media/images/{id}?account_id={}", b.id);
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

#[tokio::test]
async fn jpeg_http_uses_bytes_not_filename_and_returns_oriented_png() {
    use sha2::{Digest, Sha256};
    let (repo, a, _) = common::legacy_pair("JPEG自有", "JPEG隔离自有");
    let app = Arc::new(Application::new(repo, Arc::new(WindowsWechatProvider)));
    let token = app
        .issue_token_local(
            "JPEG上传",
            vec![a.id.clone()],
            vec!["media:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let router = werc_http_api::router(app.clone());
    let jpeg = include_bytes!("../../../tests/fixtures/owned-photo-orientation-6.jpg");
    let uri = format!("/api/v1/media/images?account_id={}", a.id);
    // Intentionally PNG filename/MIME: authenticated bytes decide the format.
    let (status, body) = request(
        router.clone(),
        "POST",
        &uri,
        &token.secret,
        multipart("file", jpeg),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["sent"], false);
    assert_eq!(
        result["data"]["image"],
        serde_json::json!({"width":640,"height":360})
    );
    assert_eq!(
        result["data"]["source_sha256"],
        hex::encode(Sha256::digest(jpeg))
    );
    let id = result["data"]["id"].as_str().unwrap();
    let download = format!("/api/v1/media/images/{id}?account_id={}", a.id);
    let (status, bytes) = request(router.clone(), "GET", &download, &token.secret, vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        bytes,
        *werc_domain::image::canonical_upload_png(jpeg).unwrap().1
    );
    assert_eq!(
        result["data"]["sha256"],
        hex::encode(Sha256::digest(&bytes))
    );
    assert!(app.sends_local(&a.id).unwrap().is_empty());
    let mut trailing = jpeg.to_vec();
    trailing.push(0);
    assert_eq!(
        request(
            router.clone(),
            "POST",
            &uri,
            &token.secret,
            multipart("file", &trailing)
        )
        .await
        .0,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(app.image_assets_local(&a.id).unwrap().len(), 1);
    assert_eq!(
        request(router, "DELETE", &download, &token.secret, vec![])
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}
