use super::*;

fn fixture() -> Fixture {
    let f = Fixture::new();
    {
        let mut c = f.provider.context.lock().unwrap();
        c.target_id = c.sender_id.clone();
    }
    f
}
fn image(f: &Fixture) -> String {
    f.application
        .upload_image_local(
            &f.account,
            include_bytes!("../../../../tests/fixtures/owned-image.png").to_vec(),
        )
        .unwrap()
        .id
}

#[test]
fn ordered_images_survive_encrypted_reopen_and_unknown_never_resubmits() {
    for count in [1, 2, 9] {
        let f = fixture();
        let ids: Vec<_> = (0..count).map(|_| image(&f)).collect();
        let draft = f
            .application
            .create_private_image_moment_local(&f.account, "", &ids, "owned-image-moment-order-01")
            .unwrap();
        assert_eq!(draft.metadata.schema_version, 10);
        assert_eq!(draft.state, SendState::AwaitingApproval);
        assert_eq!(
            f.application
                .create_private_image_moment_local(
                    &f.account,
                    "",
                    &ids,
                    "owned-image-moment-order-01"
                )
                .unwrap()
                .metadata
                .id,
            draft.metadata.id
        );
        if count > 1 {
            let mut reversed = ids.clone();
            reversed.reverse();
            assert!(
                f.application
                    .create_private_image_moment_local(
                        &f.account,
                        "",
                        &reversed,
                        "owned-image-moment-order-01"
                    )
                    .is_err()
            );
        }
        let repository =
            Arc::new(SqliteRepository::open(&f.directory.join("state.sqlite3")).unwrap());
        let app = Application::new(repository, f.provider.clone());
        let preview = app
            .send_preview_local(&f.account, &draft.metadata.id)
            .unwrap();
        let images = &preview.payload.moment.as_ref().unwrap().images;
        assert_eq!(
            images.iter().map(|a| &a.asset_id).collect::<Vec<_>>(),
            ids.iter().collect::<Vec<_>>()
        );
        for asset in images {
            use sha2::{Digest, Sha256};
            let bytes = app
                .image_preview_local(&f.account, &asset.asset_id)
                .unwrap();
            assert_eq!(hex::encode(Sha256::digest(&bytes)), asset.png_sha256);
            let disk = std::fs::read(f.directory.join("state.sqlite3")).unwrap();
            assert!(!disk.windows(bytes.len()).any(|v| v == bytes.as_slice()));
        }
        assert!(
            app.execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        f.approve(&draft);
        f.provider.mode.store(2, Ordering::SeqCst);
        let status = app
            .execute_send_local(&f.account, &draft.metadata.id)
            .unwrap();
        assert_eq!(status.state, SendState::Unknown);
        assert!(status.attempt_id.is_some());
        assert!(
            app.execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        assert!(!app.run_send_once_local().unwrap());
        for _ in 0..2 {
            assert_eq!(
                app.reconcile_send_local(&f.account, &draft.metadata.id)
                    .unwrap()
                    .state,
                SendState::ServerAccepted
            );
        }
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn image_moment_missing_tampered_expired_or_changed_assets_cancel_before_attempt() {
    for mode in 0..5 {
        let f = fixture();
        let ids = vec![image(&f), image(&f)];
        let draft = f
            .application
            .create_private_image_moment_local(
                &f.account,
                "自动化测试",
                &ids,
                "owned-image-moment-cancel-01",
            )
            .unwrap();
        f.approve(&draft);
        match mode {
            0 => f
                .application
                .delete_image_local(&f.account, &ids[1])
                .unwrap(),
            1 => {
                f.database().execute("UPDATE image_assets SET ciphertext=zeroblob(length(ciphertext)) WHERE id=?1", [&ids[1]]).unwrap();
            }
            2 => {
                f.database().execute("UPDATE image_assets SET expires_at='2000-01-01T00:00:00+00:00' WHERE id=?1", [&ids[1]]).unwrap();
            }
            3 => {
                f.database().execute("UPDATE image_assets SET metadata=json_set(metadata,'$.image.width',3) WHERE id=?1", [&ids[1]]).unwrap();
            }
            _ => f.provider.context.lock().unwrap().epoch = "changed".into(),
        }
        assert!(
            f.application
                .execute_send_local(&f.account, &draft.metadata.id)
                .is_err()
        );
        let status = f
            .repository
            .send_status(&f.account, &draft.metadata.id)
            .unwrap();
        assert_eq!(status.state, SendState::Canceled);
        assert!(status.attempt_id.is_none());
        assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn image_moment_limits_and_cross_account_assets_reject_before_draft() {
    let f = fixture();
    let id = image(&f);
    for (body, ids) in [
        ("".into(), vec![]),
        ("x".repeat(10001), vec![id.clone()]),
        ("a\0b".into(), vec![id.clone()]),
        ("自动化测试".into(), vec![id.clone(); 10]),
        ("".into(), vec!["absent".into()]),
    ] {
        assert!(
            f.application
                .create_private_image_moment_local(
                    &f.account,
                    &body,
                    &ids,
                    "owned-image-moment-invalid"
                )
                .is_err()
        );
    }
    let other = fixture();
    let id = image(&other);
    assert!(
        f.application
            .create_private_image_moment_local(&f.account, "", &[id], "owned-image-moment-account")
            .is_err()
    );
    assert!(f.application.sends_local(&f.account).unwrap().is_empty());
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn moment_only_api_token_can_create_and_cancel_both_schemas_without_message_scope() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = fixture();
    let id = image(&f);
    let token = f
        .application
        .issue_token_local(
            "自有动态权限",
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
    for with_images in [false, true] {
        let input = if with_images {
            serde_json::json!({"account_id":f.account,"text":"","visibility":"private","image_asset_ids":[id]})
        } else {
            serde_json::json!({"account_id":f.account,"text":"自动化测试","visibility":"private"})
        };
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/moments/publish")
            .header("Authorization", format!("Bearer {}", token.secret))
            .header("Content-Type", "application/json")
            .header(
                "Idempotency-Key",
                format!("owned-image-moment-api-{with_images}"),
            )
            .body(Body::from(input.to_string()))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let draft_id = value["data"]["metadata"]["id"].as_str().unwrap();
        assert_eq!(
            value["data"]["metadata"]["schema_version"],
            if with_images { 10 } else { 9 }
        );
        let request = Request::builder()
            .method("DELETE")
            .uri(format!(
                "/api/v1/outbox/{draft_id}?account_id={}",
                f.account
            ))
            .header("Authorization", format!("Bearer {}", token.secret))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(value["data"]["state"], "canceled");
        assert!(value["data"]["attempt_id"].is_null());
    }
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}
