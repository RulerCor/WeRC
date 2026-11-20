use super::*;
use werc_domain::favorites::FavoriteQuery;
#[test]
fn favorites_cursor_and_exact_copy_are_bound_to_content_and_view() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let page = f
        .app
        .live_favorites_local(
            &f.account,
            &view.view_id,
            Some(FavoriteQuery::default()),
            None,
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].anchor.local_id, "2");
    assert_eq!(page.rows[0].content.as_ref().unwrap().tags, ["关联标签"]);
    let public = serde_json::to_string(&page).unwrap();
    assert!(!public.contains(&"b".repeat(64)));
    assert!(!public.contains("cdn_data"));
    let next = f
        .app
        .live_favorites_local(
            &f.account,
            &view.view_id,
            None,
            page.next.as_deref(),
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(next.rows[0].anchor.local_id, "1");
    assert!(next.next.is_none());
    let copy = f
        .app
        .copy_favorite_local(
            &f.account,
            &view.view_id,
            &page.rows[0].item_token,
            &uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(copy.header.kind, "favorite");
    assert_eq!(copy.entry.tags, ["关联标签"]);
    assert!(copy.entry.source.unwrap().starts_with("wechat-favorite:2:"));
    assert!(
        f.app
            .live_favorites_local(
                &f.account,
                &view.view_id,
                None,
                Some(&page.rows[0].item_token),
                &uuid::Uuid::new_v4().to_string()
            )
            .is_err()
    );
    *f.reader.body.lock().unwrap() = "changed after listing".into();
    assert_eq!(
        f.app
            .copy_favorite_local(
                &f.account,
                &view.view_id,
                &page.rows[0].item_token,
                &uuid::Uuid::new_v4().to_string()
            )
            .err()
            .unwrap()
            .code,
        "CURSOR_STALE"
    );
}
#[test]
fn favorites_reject_login_change_during_read() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .live_favorites_local(
                &f.account,
                &view.view_id,
                Some(FavoriteQuery::default()),
                None,
                &uuid::Uuid::new_v4().to_string()
            )
            .is_err()
    );
}
#[tokio::test]
async fn favorites_http_enforces_scopes_and_keeps_credentials_internal() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    f.app.enable_api(true);
    for (scope, status) in [("accounts:read", 403), ("sources:read", 200)] {
        let t = f
            .app
            .issue_token_local(
                "owned-favorites",
                vec![f.account.clone()],
                vec![scope.into()],
                1,
            )
            .unwrap();
        let body = serde_json::json!({"account_id":f.account,"view_id":view.view_id,"query":{},"request_id":uuid::Uuid::new_v4().to_string()});
        let response = werc_http_api::router(f.app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/favorites/live")
                    .header("Authorization", format!("Bearer {}", t.secret))
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        if status == 200 {
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let text = String::from_utf8(bytes.to_vec()).unwrap();
            assert!(!text.contains("cdn_data"));
            assert!(text.contains("local_wechat_favorites"));
        }
    }
}
