use super::*;
fn fixture() -> (Fixture, String, String) {
    let f = Fixture::new();
    {
        let mut c = f.provider.context.lock().unwrap();
        c.target_id = c.sender_id.clone();
    }
    let video = f
        .application
        .upload_video_local(
            &f.account,
            "自动化测试.mp4",
            include_bytes!("../../../../tests/fixtures/owned-send-video.mp4").to_vec(),
        )
        .unwrap()
        .id;
    let cover = f
        .application
        .upload_image_local(
            &f.account,
            include_bytes!("../../../../tests/fixtures/owned-image.png").to_vec(),
        )
        .unwrap()
        .id;
    (f, video, cover)
}
#[test]
fn video_moment_bot_executes_once_and_recovers_same_key_after_assets_removed() {
    let (f, video, cover) = fixture();
    let token = f
        .application
        .issue_token_local(
            "自动化测试",
            vec![f.account.clone()],
            vec![
                "moments:publish".into(),
                "operations:create".into(),
                "messages:execute".into(),
            ],
            1,
        )
        .unwrap();
    f.application.enable_api(true);
    let p = f.application.authenticate(&token.secret).unwrap();
    let key = "owned-bot-video-moment-001";
    let created = f
        .application
        .create_video_moment(
            &p,
            &f.account,
            "自动化测试",
            &video,
            &cover,
            MomentVisibility::Private,
            key,
        )
        .unwrap();
    assert_eq!(created.state, SendState::Queued);
    assert!(f.application.run_send_once_local().unwrap());
    let repo = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(repo, f.provider.clone());
    app.enable_api(true);
    app.delete_file_local(&f.account, &video).unwrap();
    app.delete_image_local(&f.account, &cover).unwrap();
    f.provider.context.lock().unwrap().epoch = "reconnected".into();
    let same = app
        .create_video_moment(
            &p,
            &f.account,
            "自动化测试",
            &video,
            &cover,
            MomentVisibility::Private,
            key,
        )
        .unwrap();
    assert_eq!(same.metadata.id, created.metadata.id);
    assert_eq!(same.state, SendState::Unknown);
    assert!(!app.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        app.create_video_moment(
            &p,
            &f.account,
            "自动化测试",
            &video,
            &cover,
            MomentVisibility::Public,
            key
        )
        .unwrap_err()
        .code,
        "IDEMPOTENCY_CONFLICT"
    );
}
#[test]
fn video_moment_reopen_binds_assets_visibility_and_never_resends_unknown() {
    let (f, video, cover) = fixture();
    let key = "owned-video-moment-0001";
    let draft = f
        .application
        .create_video_moment_local(
            &f.account,
            "自动化测试",
            &video,
            &cover,
            MomentVisibility::Private,
            key,
        )
        .unwrap();
    assert_eq!(draft.metadata.schema_version, 12);
    assert!(
        f.application
            .create_video_moment_local(
                &f.account,
                "自动化测试",
                &video,
                &cover,
                MomentVisibility::Public,
                key
            )
            .is_err()
    );
    let repo = Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
    let app = Application::new(repo, f.provider.clone());
    let preview = app
        .send_preview_local(&f.account, &draft.metadata.id)
        .unwrap();
    let moment = preview.payload.moment.as_ref().unwrap();
    assert_eq!(moment.video.as_ref().unwrap().file.asset_id, video);
    assert_eq!(moment.images[0].asset_id, cover);
    assert_eq!(moment.visibility, MomentVisibility::Private);
    f.approve(&draft);
    assert_eq!(
        app.execute_send_local(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert!(
        app.execute_send_local(&f.account, &draft.metadata.id)
            .is_err()
    );
    assert!(!app.run_send_once_local().unwrap());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn video_moment_missing_video_or_cover_cancels_before_consuming_attempt() {
    for remove_video in [true, false] {
        let (f, video, cover) = fixture();
        let draft = f
            .application
            .create_video_moment_local(
                &f.account,
                "",
                &video,
                &cover,
                MomentVisibility::Public,
                "owned-video-deletion-01",
            )
            .unwrap();
        f.approve(&draft);
        if remove_video {
            f.application.delete_file_local(&f.account, &video).unwrap();
        } else {
            f.application
                .delete_image_local(&f.account, &cover)
                .unwrap();
        }
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        let status = f
            .application
            .send_preview_local(&f.account, &draft.metadata.id)
            .unwrap()
            .status;
        assert_eq!(status.state, SendState::Canceled);
        assert!(status.attempt_id.is_none());
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn video_moment_api_preserves_schema_and_requires_single_cover() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let (f, video, cover) = fixture();
    let token = f
        .application
        .issue_token_local(
            "自动化测试",
            vec![f.account.clone()],
            vec![
                "moments:publish".into(),
                "operations:create".into(),
                "operations:read".into(),
            ],
            1,
        )
        .unwrap();
    f.application.enable_api(true);
    let router = werc_http_api::router(f.application.clone());
    for count in [0, 1, 2] {
        let input = serde_json::json!({"account_id":f.account,"text":"自动化测试","visibility":"private","video_asset_id":video,"image_asset_ids":vec![cover.clone();count]});
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/moments/publish")
            .header("Authorization", format!("Bearer {}", token.secret))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", format!("owned-video-api-{count:04}"))
            .body(Body::from(input.to_string()))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), if count == 1 { 200 } else { 400 });
        if count == 1 {
            let data: serde_json::Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert_eq!(data["data"]["metadata"]["schema_version"], 12);
            assert_eq!(data["data"]["state"], "awaiting_approval");
        }
    }
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
