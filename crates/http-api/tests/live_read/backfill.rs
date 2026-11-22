use super::*;

#[test]
fn automatic_backfill_resumes_after_reconnect_and_preserves_source_identity() {
    let f = Fixture::new();
    assert_eq!(
        f.app
            .backfill_status_local(&f.account)
            .unwrap()
            .source_count,
        0
    );
    f.prepare().unwrap();
    let first = f.app.step_backfill_local(&f.account).unwrap();
    assert_eq!(first.rows, 60);
    assert_eq!(first.pages, 1);
    let page = f.app.backfill_page_local(&f.account, 0, 1).unwrap();
    assert_eq!(page.next, 1);
    assert!(
        page.data[0]
            .rows
            .iter()
            .all(|r| r.message.metadata.is_none())
    );
    let first_id = page.data[0].rows[0].id.clone();
    f.reader.context.lock().unwrap().epoch = "owned-backfill-reconnected".into();
    let app = Application::new(f.repo.clone(), f.reader.clone());
    let mut current = app.backfill_status_local(&f.account).unwrap();
    for _ in 0..30 {
        if current.pending_sources == 0 {
            break;
        }
        current = app.step_backfill_local(&f.account).unwrap();
    }
    assert_eq!(current.pending_sources, 0);
    assert_eq!(current.rows, 500);
    assert!(!current.historical_complete);
    assert!(current.observed_through.is_some());
    assert_eq!(
        app.backfill_page_local(&f.account, 0, 1).unwrap().data[0].rows[0].id,
        first_id
    );
    let mut ids = std::collections::BTreeSet::new();
    let mut after = 0;
    loop {
        let page = app.backfill_page_local(&f.account, after, 2).unwrap();
        for batch in page.data {
            for row in batch.rows {
                assert!(ids.insert(row.id));
            }
        }
        after = page.next;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(ids.len(), 500);
    assert!(app.backfill_page_local(&f.account, after + 1, 1).is_err());
    assert_eq!(
        app.step_backfill_local(&f.account).unwrap().pages,
        current.pages
    );
    let events = f.repo.events(0, &f.account, 100).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "receive.recovered")
            .count() as u64,
        current.pages
    );
    assert!(app.receive_cursor_local(&f.account).unwrap().is_none());
}

#[test]
fn automatic_backfill_source_change_keeps_checkpoint_and_reports_error() {
    let f = Fixture::new();
    f.prepare().unwrap();
    let before = f.app.step_backfill_local(&f.account).unwrap();
    *f.reader.salt.lock().unwrap() = [0x91; 16];
    let after = f.app.step_backfill_local(&f.account).unwrap();
    assert_eq!(before.pages, after.pages);
    assert_eq!(before.rows, after.rows);
    assert_eq!(after.errors, vec!["LIVE_SOURCE_REPLACED"]);
    assert!(after.pending_sources > 0);
    assert!(
        !f.app
            .backfill_page_local(&f.account, 0, 1)
            .unwrap()
            .data
            .is_empty()
    );
}

#[tokio::test]
async fn recovered_http_requires_scope_and_uses_durable_page_cursor() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    f.prepare().unwrap();
    f.app.step_backfill_local(&f.account).unwrap();
    f.app.enable_api(true);
    for (scopes, expected) in [
        (vec!["accounts:read".into()], 403),
        (vec!["messages:read".into()], 200),
    ] {
        let token = f
            .app
            .issue_token_local("owned-backfill", vec![f.account.clone()], scopes, 1)
            .unwrap();
        let response = werc_http_api::router(f.app.clone())
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/messages/recovered?account_id={}&after=0",
                        f.account
                    ))
                    .header("Authorization", format!("Bearer {}", token.secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
        if expected == 200 {
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["next"], 1);
            assert_eq!(body["data"][0]["rows"].as_array().unwrap().len(), 60);
            assert_eq!(body["status"]["historical_complete"], false);
        }
    }
}

#[test]
fn automatic_discovery_enrolls_without_creating_or_replacing_a_browse_view() {
    let f = Fixture::new();
    let view = f.prepare().unwrap();
    // A second application has no foreground view but can reuse protected source keys.
    let app = Application::new(f.repo.clone(), f.reader.clone());
    let database = f.database.with_file_name("message_1.db");
    std::fs::copy(&f.database, &database).unwrap();
    let database = database.to_str().unwrap().to_owned();
    *f.reader.auto_sources.lock().unwrap() = vec![database];
    app.discover_backfill_sources_local(&f.account).unwrap();
    assert_eq!(
        app.backfill_status_local(&f.account).unwrap().source_count,
        2
    );
    let pages = app.step_backfill_local(&f.account).unwrap().pages;
    assert_eq!(pages, 1);
    assert_eq!(
        f.app
            .live_filehelper_page_local(&f.account, None, 60)
            .unwrap()
            .view_id,
        view.view_id
    );
}

#[test]
fn automatic_view_uses_owned_root_deduplicates_and_merges_all_selected_shards() {
    use werc_application::backfill::AutomaticLivePage;
    let f = Fixture::new();
    f.reader.spread_time.store(true, Ordering::SeqCst);
    f.prepare().unwrap();
    assert_eq!(
        f.app
            .prepare_automatic_live_local(&f.account, "owned-auto-empty", false)
            .err()
            .unwrap()
            .code,
        "SOURCE_NOT_FOUND"
    );
    let second = f.database.with_file_name("message_1.db");
    std::fs::copy(&f.database, &second).unwrap();
    *f.reader.auto_sources.lock().unwrap() = vec![
        f.database.to_str().unwrap().into(),
        second.to_str().unwrap().into(),
        second.to_str().unwrap().into(),
    ];
    let page = f
        .app
        .prepare_automatic_live_local(&f.account, "owned-auto-merged", false)
        .unwrap();
    let AutomaticLivePage::Multiple(mut page) = page else {
        panic!("Expected both distinct sources");
    };
    assert_eq!(page.source_count, 2);
    let mut observed = std::collections::BTreeSet::new();
    loop {
        for row in &page.rows {
            observed.insert(row.source_index);
        }
        let Some(cursor) = page.older else {
            break;
        };
        page = f
            .app
            .live_filehelper_multi_page_local(&f.account, Some((&cursor, LiveDirection::Older)), 60)
            .unwrap();
    }
    assert_eq!(observed, std::collections::BTreeSet::from([1, 2]));
    let single = f
        .app
        .prepare_automatic_live_local(&f.account, "owned-auto-tools", true)
        .unwrap();
    assert!(matches!(single, AutomaticLivePage::Single(_)));
}

#[test]
fn automatic_view_does_not_choose_an_arbitrary_unowned_root() {
    let f = Fixture::new();
    let mut paths = Vec::new();
    for name in ["ambiguous-a", "ambiguous-b"] {
        let folder = f.directory.join(name).join("db_storage/message");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("message_0.db");
        std::fs::copy(&f.database, &path).unwrap();
        paths.push(path.to_str().unwrap().into());
    }
    *f.reader.auto_sources.lock().unwrap() = paths;
    assert_eq!(
        f.app
            .prepare_automatic_live_local(&f.account, "owned-auto-ambiguous", false)
            .err()
            .unwrap()
            .code,
        "SOURCE_SELECTION_REQUIRED"
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn recovery_api_archives_verified_sources_resumes_and_exports_through_mcp() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn call(app: &Arc<Application>, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
        let response = werc_http_api::router(app.clone()).oneshot(Request::builder()
            .method(method).uri(path).header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let f = Fixture::new();
    let app = &f.app;
    app.enable_api(true);
    let writer = app.issue_token_local("owned writer", vec![f.account.clone()], vec!["messages:read".into(), "exports:create".into()], 1).unwrap();
    let reader = app.issue_token_local("owned reader", vec![f.account.clone()], vec!["messages:read".into()], 1).unwrap();
    let recovery = "/api/v1/messages/recovery";
    let start = json!({"action":"start","account_id":f.account});
    *f.reader.auto_sources.lock().unwrap() = vec![f.database.to_str().unwrap().to_owned()];
    assert_eq!(call(app, &reader.secret, "POST", recovery, start.clone()).await.0, 403);
    assert_eq!(call(app, &writer.secret, "POST", recovery, start.clone()).await.0, 409);
    f.prepare().unwrap();
    let (code, progress) = call(app, &reader.secret, "GET", &format!("/api/v1/messages/recovered/status?account_id={}", f.account), Value::Null).await;
    assert_eq!(code, 200);
    assert_eq!(progress["source_count"], 1);
    assert!(progress.get("data").is_none());
    assert_eq!(call(app, &reader.secret, "GET", "/api/v1/messages/recovered/status?account_id=other-account", Value::Null).await.0, 403);
    assert_eq!(call(app, &writer.secret, "POST", recovery, start.clone()).await.0, 409);
    for _ in 0..32 {
        if app.step_backfill_local(&f.account).unwrap().pending_sources == 0 { break; }
    }
    let (code, mut current) = call(app, &writer.secret, "POST", recovery, start).await;
    assert_eq!(code, 200, "{current}");
    assert_eq!(current["source_count"], 1);
    let id = current["id"].as_str().unwrap().to_owned();
    for i in 0..32 {
        if current["complete_selected_sources"] == true { break; }
        let (code, next) = call(app, &writer.secret, "POST", recovery, json!({"action":"step","account_id":f.account,"id":id,"request_id":format!("owned-archive-step-{i:04}")})).await;
        assert_eq!(code, 200, "{next}");
        current = next;
    }
    assert_eq!(current["complete_selected_sources"], true);
    assert_eq!(current["rows"], 500);
    for (path, body) in [
        (recovery, json!({"action":"status","account_id":f.account,"id":id})),
        ("/api/v1/messages/recovery/export", json!({"account_id":f.account,"id":id,"format":"json","request_id":"owned-export-via-mcp-0001"})),
    ] {
        let (code, output) = call(app, &writer.secret, "POST", "/mcp/read", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"api_read","arguments":{"method":"POST","path":path,"body":body}}})).await;
        assert_eq!(code, 200);
        assert_eq!(output["result"]["isError"], false, "{output}");
        assert_eq!(output["result"]["structuredContent"]["status"], 200);
    }
}
