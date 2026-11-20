use super::*;
use werc_application::live_export::*;
#[test]
fn live_message_export_reads_all_pages_keeps_sources_and_filters_without_replacing_view() {
    let f = Fixture::new();
    let parent = f
        .app
        .prepare_live_filehelper_multi_local(
            &f.account,
            &two_sources(&f),
            "owned-export-parent",
            true,
        )
        .unwrap();
    let selection = LiveExportSelection {
        username: "owned-A".into(),
        query: LiveMessageQuery::default(),
    };
    let output = f
        .app
        .export_live_messages_local(
            &f.account,
            &parent.view_id,
            &selection,
            LiveExportFormat::Json,
            "owned-export-full",
            Ok,
        )
        .unwrap();
    assert_eq!(output.summary.rows, 500);
    assert_eq!(output.summary.pages, 4);
    assert_eq!(output.summary.source_count, 2);
    assert_eq!(output.summary.media_written, 0);
    let json: serde_json::Value = serde_json::from_slice(&output.bytes).unwrap();
    let rows = json["rows"].as_array().unwrap();
    assert_eq!(rows[0]["source_index"], 1);
    assert_eq!(rows[0]["message"]["local_id"], "1");
    assert_eq!(rows[250]["source_index"], 2);
    assert_eq!(rows[250]["message"]["local_id"], "1");
    assert_eq!(rows[499]["message"]["server_id"], "9007199254741242");
    assert!(rows.iter().all(|r| r["message"].get("metadata").is_none()));
    let page = f
        .app
        .live_multi_conversation_local(
            &f.account,
            werc_application::live_conversation::ConversationCommand::Open {
                parent_view_id: parent.view_id.clone(),
                username: "owned-A".into(),
            },
            "owned-export-open",
        )
        .unwrap()
        .unwrap();
    let mut filtered = LiveExportSelection {
        username: "owned-A".into(),
        query: LiveMessageQuery::default(),
    };
    filtered.query.server_id = Some("9007199254741002".into());
    for format in [
        LiveExportFormat::Csv,
        LiveExportFormat::Txt,
        LiveExportFormat::Xlsx,
        LiveExportFormat::Html,
        LiveExportFormat::Md,
    ] {
        let output = f
            .app
            .export_live_messages_local(
                &f.account,
                &parent.view_id,
                &filtered,
                format,
                "owned-export-filter",
                Ok,
            )
            .unwrap();
        assert_eq!(output.summary.rows, 2);
        assert!(!output.bytes.is_empty());
    }
    let more = f
        .app
        .live_multi_conversation_local(
            &f.account,
            werc_application::live_conversation::ConversationCommand::Page {
                view_id: page.messages.view_id.clone(),
                cursor: page.messages.older.unwrap(),
                direction: LiveDirection::Older,
            },
            "owned-export-view-retained",
        )
        .unwrap()
        .unwrap();
    assert_eq!(more.messages.view_id, page.messages.view_id);
    assert!(f.repo.sends(&f.account).unwrap().is_empty());
}

#[test]
fn live_message_export_cancellation_and_source_changes_never_invoke_writer() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let selection = LiveExportSelection {
        username: "owned-A".into(),
        query: LiveMessageQuery::default(),
    };
    let written = AtomicBool::new(false);
    f.app.cancel_browse_local("owned-directory-cancel").unwrap();
    assert_eq!(
        f.app
            .export_live_messages_local(
                &f.account,
                &parent.view_id,
                &selection,
                LiveExportFormat::Csv,
                "owned-directory-cancel",
                |_| {
                    written.store(true, Ordering::SeqCst);
                    Ok(())
                }
            )
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    f.reader.change.store(true, Ordering::SeqCst);
    assert!(
        f.app
            .export_live_messages_local(
                &f.account,
                &parent.view_id,
                &selection,
                LiveExportFormat::Json,
                "owned-directory-changed",
                |_| {
                    written.store(true, Ordering::SeqCst);
                    Ok(())
                }
            )
            .is_err()
    );
    assert!(!written.load(Ordering::SeqCst));
}
#[tokio::test]
async fn live_message_export_http_requires_read_and_export_scopes_and_returns_attachment() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str| {
        Request::builder().method("POST").uri("/api/v1/messages/live/export").header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json").body(Body::from(serde_json::json!({"account_id":f.account,"view_id":parent.view_id,"selection":{"username":"owned-A","query":{}},"format":"json","request_id":"owned-directory-http"}).to_string())).unwrap()
    };
    for scope in ["messages:read", "exports:create"] {
        let token = f
            .app
            .issue_token_local(
                "owned missing scope",
                vec![f.account.clone()],
                vec![scope.into()],
                1,
            )
            .unwrap();
        assert_eq!(
            router
                .clone()
                .oneshot(request(&token.secret))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let token = f
        .app
        .issue_token_local(
            "owned directory export",
            vec![f.account.clone()],
            vec!["messages:read".into(), "exports:create".into()],
            1,
        )
        .unwrap();
    let response = router.oneshot(request(&token.secret)).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-werc-records"], "250");
    assert_eq!(response.headers()["x-werc-pages"], "2");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment")
    );
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["rows"].as_array().unwrap().len(), 250);
}
