use super::*;
use werc_domain::special_records::*;
fn query() -> RecordQuery {
    RecordQuery {
        source: RecordSource::FriendRequest,
        text: String::new(),
        from: None,
        until: None,
    }
}
#[test]
fn special_records_bound_pagination_full_exports_and_identity_change() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    let id = || uuid::Uuid::new_v4().to_string();
    let first = f
        .app
        .live_special_records_local(&f.account, &view.view_id, Some(query()), None, &id())
        .unwrap();
    assert_eq!(first.rows[0].anchor.row_id, "2");
    let next = f
        .app
        .live_special_records_local(
            &f.account,
            &view.view_id,
            None,
            first.next.as_deref(),
            &id(),
        )
        .unwrap();
    assert_eq!(next.rows[0].anchor.row_id, "1");
    assert!(next.next.is_none());
    assert!(
        f.app
            .live_special_records_local(
                &f.account,
                &view.view_id,
                Some(query()),
                first.next.as_deref(),
                &id()
            )
            .is_err()
    );
    for format in ["json", "csv", "txt", "xlsx"] {
        f.app
            .export_special_records_local(
                &f.account,
                &view.view_id,
                query(),
                &id(),
                format,
                |bytes, count| {
                    assert_eq!(count, 2);
                    assert!(!bytes.is_empty());
                    if format == "json" {
                        let data: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                        assert_eq!(data["rows"].as_array().unwrap().len(), 2);
                    }
                    if format == "csv" {
                        assert!(std::str::from_utf8(bytes).unwrap().contains("'=自动化测试"));
                    }
                    if format == "xlsx" {
                        assert!(bytes.starts_with(b"PK"));
                    }
                    Ok(())
                },
            )
            .unwrap();
    }
    f.reader.change.store(true, Ordering::SeqCst);
    let published = AtomicBool::new(false);
    assert!(
        f.app
            .export_special_records_local(
                &f.account,
                &view.view_id,
                query(),
                &id(),
                "json",
                |_, _| {
                    published.store(true, Ordering::SeqCst);
                    Ok(())
                }
            )
            .is_err()
    );
    assert!(!published.load(Ordering::SeqCst));
}
#[tokio::test]
async fn special_records_http_separates_read_and_export_permissions() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    f.app.enable_api(true);
    for (scopes, tail, status) in [
        (vec!["accounts:read"], "", 403),
        (vec!["sources:read"], "", 200),
        (vec!["sources:read"], "/export", 403),
        (vec!["sources:read", "exports:create"], "/export", 200),
    ] {
        let token = f
            .app
            .issue_token_local(
                "owned-records",
                vec![f.account.clone()],
                scopes.into_iter().map(String::from).collect(),
                1,
            )
            .unwrap();
        let mut body = serde_json::json!({"account_id":f.account,"view_id":view.view_id,"query":query(),"request_id":uuid::Uuid::new_v4().to_string()});
        if tail == "/export" {
            body["format"] = "json".into();
        }
        let response = werc_http_api::router(f.app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/records/live{tail}"))
                    .header("Authorization", format!("Bearer {}", token.secret))
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
    }
}
