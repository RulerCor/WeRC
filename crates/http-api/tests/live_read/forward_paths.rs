use super::*;
use werc_application::conversation_media::ConversationMediaKind;

fn nested_images(depth: usize) -> String {
    let original = owned_forward_image_xml();
    let mut record = original
        .split("<![CDATA[")
        .nth(1)
        .unwrap()
        .split("]]>")
        .next()
        .unwrap()
        .to_owned();
    for _ in 1..depth {
        record = format!(
            "<recordinfo><datalist><dataitem datatype=\"1\"><datadesc>owned sibling</datadesc></dataitem><dataitem datatype=\"17\"><recordxml>{record}</recordxml></dataitem></datalist></recordinfo>"
        );
    }
    format!(
        "<msg><appmsg><type>19</type><recorditem><![CDATA[{record}]]></recorditem></appmsg></msg>"
    )
}

#[test]
fn nested_images_reopen_and_delete_for_single_and_multi_source_views() {
    for multi in [false, true] {
        for depth in [1, 2, 4] {
            let f = Fixture::new();
            *f.reader.body.lock().unwrap() = nested_images(depth);
            let selection = selected_media(&f, multi);
            let gap = f.repo.receive_cursor(&f.account).unwrap();
            let asset = f
                .app
                .save_conversation_media_path_local(
                    &f.account,
                    &selection,
                    None,
                    Some(&vec![1; depth]),
                    "owned-nested-image-path",
                )
                .unwrap();
            assert_eq!(asset.reference.kind, ConversationMediaKind::Image);
            let content = f
                .app
                .conversation_media_content_local(&f.account, &asset.reference)
                .unwrap();
            assert_eq!(content.mime, "image/png");
            let expected = werc_domain::image::canonical_png(include_bytes!(
                "../../../../tests/fixtures/owned-image.png"
            ))
            .unwrap()
            .1;
            assert_eq!(content.file.bytes, expected);
            let db = f.directory.join("state.sqlite3");
            let account = f.account.clone();
            let reader = f.reader.clone();
            let _reopen_directory=f._cleanup.clone();
            drop(f);
            let repo = Arc::new(SqliteRepository::open(&db).unwrap());
            let app = Application::new(repo.clone(), reader);
            assert_eq!(
                app.conversation_media_content_local(&account, &asset.reference)
                    .unwrap()
                    .file
                    .bytes,
                expected
            );
            assert_eq!(repo.receive_cursor(&account).unwrap(), gap);
            assert!(repo.sends(&account).unwrap().is_empty());
            app.delete_conversation_media_local(&account, &asset.reference)
                .unwrap();
            assert!(
                app.conversation_media_content_local(&account, &asset.reference)
                    .is_err()
            );
        }
    }
}

#[test]
fn nested_image_invalid_path_integrity_and_source_leave_no_asset() {
    for multi in [false, true] {
        for mode in 0..9 {
            let f = Fixture::new();
            let xml = nested_images(2);
            *f.reader.body.lock().unwrap() = if mode == 5 {
                xml.replace("22f9f424c04f42866b833a812ebe4605", &"0".repeat(32))
            } else {
                xml
            };
            let mut selection = selected_media(&f, multi);
            if mode == 6 {
                selection.source = Some("wrong-source".into());
            }
            if mode == 7 {
                selection.view_id = "wrong-view".into();
            }
            if mode == 8 {
                f.app
                    .cancel_browse_local("owned-nested-image-reject")
                    .unwrap();
            }
            let path = match mode {
                0 => vec![],
                1 => vec![1],
                2 => vec![0, 1],
                3 => vec![1, 100],
                4 => vec![1; 5],
                _ => vec![1, 1],
            };
            assert!(
                f.app
                    .save_conversation_media_path_local(
                        &f.account,
                        &selection,
                        None,
                        Some(&path),
                        "owned-nested-image-reject"
                    )
                    .is_err()
            );
            assert!(f.app.file_assets_local(&f.account).unwrap().is_empty());
            assert!(f.repo.sends(&f.account).unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn nested_image_http_path_preserves_scope_exclusivity_and_no_credentials() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    *f.reader.body.lock().unwrap() = nested_images(2);
    let selection = selected_media(&f, true);
    let token = f
        .app
        .issue_token_local(
            "owned nested",
            vec![f.account.clone()],
            vec!["messages:read".into(), "media:write".into()],
            1,
        )
        .unwrap();
    let read = f
        .app
        .issue_token_local(
            "owned read",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let body = serde_json::json!({"account_id":f.account,"selection":{"view_id":selection.view_id,"local_id":selection.local_id,"source":selection.source},"forward_path":[1,1],"request_id":"owned-nested-image-http"});
    let req = |secret: &str, v: &serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/messages/live/conversation/media")
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap()
    };
    assert_eq!(
        router
            .clone()
            .oneshot(req(&read.secret, &body))
            .await
            .unwrap()
            .status(),
        403
    );
    for field in ["aes_key", "media_type", "file_name", "database"] {
        let mut v = body.clone();
        v[field] = "untrusted".into();
        assert_eq!(
            router
                .clone()
                .oneshot(req(&token.secret, &v))
                .await
                .unwrap()
                .status(),
            422
        );
    }
    let mut both = body.clone();
    both["forward_index"] = 1.into();
    assert_eq!(
        router
            .clone()
            .oneshot(req(&token.secret, &both))
            .await
            .unwrap()
            .status(),
        400
    );
    let response = router.oneshot(req(&token.secret, &body)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["reference"]["kind"], "image");
    assert!(!String::from_utf8_lossy(&bytes).contains("cdndatakey"));
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}
