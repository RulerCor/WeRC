use super::*;
fn selection() -> LiveCalendarSelection {
    LiveCalendarSelection {
        username: "owned-A".into(),
        days: vec![
            LiveCalendarDay {
                from_timestamp: 1_699_999_900,
                until_timestamp: 1_700_000_300,
            },
            LiveCalendarDay {
                from_timestamp: 1_700_000_300,
                until_timestamp: 1_700_086_700,
            },
        ],
    }
}
#[test]
fn calendar_combines_sources_and_rejects_changed_parent() {
    for multi in [false, true] {
        let f = Fixture::new();
        let parent = if multi {
            f.app
                .prepare_live_filehelper_multi_local(
                    &f.account,
                    &two_sources(&f),
                    "owned-calendar-parent",
                    true,
                )
                .unwrap()
                .view_id
        } else {
            f.prepare().unwrap().view_id
        };
        let result = f
            .app
            .live_calendar_local(&f.account, &parent, &selection(), "owned-calendar-count")
            .unwrap();
        assert_eq!(
            result.counts,
            if multi { vec![500, 0] } else { vec![250, 0] }
        );
        assert_eq!(result.source_snapshot_atomic, !multi);
        f.reader.change.store(true, Ordering::SeqCst);
        assert!(
            f.app
                .live_calendar_local(&f.account, &parent, &selection(), "owned-calendar-changed")
                .is_err()
        );
        assert!(f.repo.sends(&f.account).unwrap().is_empty());
    }
}
#[test]
fn calendar_cancel_interrupts_the_source_read() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    f.reader.wait.store(true, Ordering::SeqCst);
    let reads = f.reader.reads.load(Ordering::SeqCst);
    let app = f.app.clone();
    let account = f.account.clone();
    let task = std::thread::spawn(move || {
        app.live_calendar_local(
            &account,
            &parent.view_id,
            &selection(),
            "owned-calendar-cancel",
        )
    });
    let start = std::time::Instant::now();
    while f.reader.reads.load(Ordering::SeqCst) == reads {
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    f.app.cancel_browse_local("owned-calendar-cancel").unwrap();
    assert_eq!(task.join().unwrap().err().unwrap().code, "QUERY_CANCELED");
}

#[tokio::test]
async fn calendar_http_enforces_scope_account_and_revocation_before_output() {
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
        Request::builder().method("POST").uri("/api/v1/messages/live/calendar").header("Authorization",format!("Bearer {secret}")).header("Content-Type","application/json").body(Body::from(serde_json::json!({"account_id":account,"view_id":parent.view_id,"selection":selection(),"request_id":"owned-search-http-0001"}).to_string())).unwrap()
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
    assert_eq!(data["counts"], serde_json::json!([250, 0]));
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
