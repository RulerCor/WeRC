use super::*;
use werc_application::live_conversation::ConversationCommand as C;

#[tokio::test]
async fn cross_search_http_enforces_scope_account_and_revocation_before_output() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let token = f
        .app
        .issue_token_local(
            "owned search",
            vec![f.account.clone()],
            vec!["messages:read".into()],
            1,
        )
        .unwrap();
    let wrong = f
        .app
        .issue_token_local(
            "owned wrong",
            vec![f.account.clone()],
            vec!["sessions:read".into()],
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |secret: &str, account: &str| {
        Request::builder().method("POST").uri("/api/v1/messages/live/search").header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json").body(Body::from(serde_json::json!({"account_id":account,"view_id":parent.view_id,"query":{},"request_id":"owned-search-http-0001"}).to_string())).unwrap()
    };
    for (secret, account) in [(&*wrong.secret, &*f.account), (&*token.secret, "other")] {
        assert_eq!(
            router
                .clone()
                .oneshot(request(secret, account))
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = router
        .clone()
        .oneshot(request(&token.secret, &f.account))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let data: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(data["rows"].as_array().unwrap().len(), 60);
    f.reader.wait.store(true, Ordering::SeqCst);
    let reads = f.reader.reads.load(Ordering::SeqCst);
    let future = tokio::spawn(router.oneshot(request(&token.secret, &f.account)));
    let started = std::time::Instant::now();
    while reads == f.reader.reads.load(Ordering::SeqCst) {
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    f.app.revoke_token_local(&token.info.id).unwrap();
    f.reader.wait.store(false, Ordering::SeqCst);
    let response = future.await.unwrap().unwrap();
    assert_eq!(response.status(), 401);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("owned-message"));
}

#[test]
fn cross_search_pages_all_conversations_sources_and_opens_exact_authenticated_context() {
    for multi in [false, true] {
        let f = Fixture::new();
        let parent = if multi {
            f.app
                .prepare_live_filehelper_multi_local(
                    &f.account,
                    &two_sources(&f),
                    "owned-search-parent",
                    true,
                )
                .unwrap()
                .view_id
        } else {
            f.prepare().unwrap().view_id
        };
        let mut page = f
            .app
            .search_live_messages_local(
                &f.account,
                &parent,
                Some(LiveMessageQuery::default()),
                None,
                "owned-search-first",
            )
            .unwrap();
        let token = page.rows[40].result_token.clone();
        let local = page.rows[40].message.local_id.clone();
        assert!(
            f.app
                .search_live_messages_local(
                    &f.account,
                    &parent,
                    None,
                    Some(&token),
                    "owned-wrong-purpose"
                )
                .is_err()
        );
        let scan = page.next.clone().unwrap();
        let command = C::OpenSearch {
            parent_view_id: parent.clone(),
            result_token: scan,
        };
        assert!(if multi {
            f.app
                .live_multi_conversation_local(&f.account, command, "owned-wrong-open")
                .is_err()
        } else {
            f.app
                .live_conversation_local(&f.account, command, "owned-wrong-open")
                .is_err()
        });
        let command = C::OpenSearch {
            parent_view_id: parent.clone(),
            result_token: token.clone(),
        };
        if multi {
            let context = f
                .app
                .live_multi_conversation_local(&f.account, command, "owned-search-context")
                .unwrap()
                .unwrap();
            let focus = context.focus.unwrap();
            assert_eq!(focus.local_id, local);
            assert!(
                context.messages.rows.iter().any(
                    |r| r.message.local_id == local && Some(&r.source) == focus.source.as_ref()
                )
            );
        } else {
            let context = f
                .app
                .live_conversation_local(&f.account, command, "owned-search-context")
                .unwrap()
                .unwrap();
            assert_eq!(context.focus.unwrap().local_id, local);
            assert_eq!(context.messages.rows[30].local_id, local);
        }
        let mut seen = std::collections::BTreeSet::new();
        loop {
            for row in &page.rows {
                assert!(seen.insert((
                    row.source_index,
                    row.username.clone(),
                    row.message.local_id.clone()
                )));
                assert!(row.message.metadata.is_none());
            }
            let Some(next) = page.next else { break };
            page = f
                .app
                .search_live_messages_local(
                    &f.account,
                    &parent,
                    None,
                    Some(&next),
                    "owned-search-next",
                )
                .unwrap();
        }
        assert_eq!(seen.len(), if multi { 1000 } else { 500 });
        let mut broken = token.clone().into_bytes();
        broken[20] = if broken[20] == b'0' { b'1' } else { b'0' };
        let command = C::OpenSearch {
            parent_view_id: parent.clone(),
            result_token: String::from_utf8(broken).unwrap(),
        };
        assert!(if multi {
            f.app
                .live_multi_conversation_local(&f.account, command, "owned-tampered-hit")
                .is_err()
        } else {
            f.app
                .live_conversation_local(&f.account, command, "owned-tampered-hit")
                .is_err()
        });
        f.reader.context.lock().unwrap().epoch = "owned-reconnected".into();
        assert!(
            f.app
                .search_live_messages_local(
                    &f.account,
                    &parent,
                    Some(LiveMessageQuery::default()),
                    None,
                    "owned-stale-search"
                )
                .is_err()
        );
        assert!(f.repo.sends(&f.account).unwrap().is_empty());
    }
}

#[test]
fn cross_search_keeps_query_over_continuation_and_honors_cancel() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let mut query = LiveMessageQuery::default();
    query.text = Some("owned-friend".into());
    let mut page = f
        .app
        .search_live_messages_local(
            &f.account,
            &parent.view_id,
            Some(query),
            None,
            "owned-filter-start",
        )
        .unwrap();
    let mut count = 0;
    loop {
        assert_eq!(page.query.text.as_deref(), Some("owned-friend"));
        for row in &page.rows {
            assert_eq!(row.username, "owned-friend");
            count += 1;
        }
        let Some(next) = page.next else { break };
        page = f
            .app
            .search_live_messages_local(
                &f.account,
                &parent.view_id,
                None,
                Some(&next),
                "owned-filter-next",
            )
            .unwrap();
    }
    assert_eq!(count, 250);
    f.app.cancel_browse_local("owned-search-cancel").unwrap();
    let reads = f.reader.reads.load(Ordering::SeqCst);
    assert_eq!(
        f.app
            .search_live_messages_local(
                &f.account,
                &parent.view_id,
                Some(LiveMessageQuery::default()),
                None,
                "owned-search-cancel"
            )
            .err()
            .unwrap()
            .code,
        "QUERY_CANCELED"
    );
    assert_eq!(f.reader.reads.load(Ordering::SeqCst), reads);
}
