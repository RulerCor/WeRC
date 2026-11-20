use super::*;

fn client(f: &Fixture, execute: bool) -> (werc_domain::IssuedToken, werc_domain::Principal) {
    let mut scopes = vec![
        "messages:send",
        "messages:send_image",
        "messages:send_file",
        "messages:send_video",
        "messages:send_emoticon",
        "messages:send_voice",
        "operations:create",
        "operations:read",
        "media:read",
        "accounts:read",
        "messages:read",
    ];
    if execute {
        scopes.push("messages:execute");
    }
    let token = f
        .application
        .issue_token_local(
            "自动化测试客户端",
            vec![f.account.clone()],
            scopes.into_iter().map(str::to_owned).collect(),
            1,
        )
        .unwrap();
    f.application.enable_api(true);
    let p = f.application.authenticate(&token.secret).unwrap();
    (token, p)
}

#[test]
fn client_execution_is_once_and_retry_survives_connection_change() {
    let f = Fixture::new();
    let (_, p) = client(&f, true);
    let create = |text: &str| {
        f.application.create_text_send(
            &p,
            &f.account,
            "synthetic-private-recipient-identity",
            text,
            "owned-bot-once-0001",
        )
    };
    let queued = create("自动化测试：客户端一次授权").unwrap();
    assert_eq!(queued.state, SendState::Queued);
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        create("自动化测试：客户端一次授权").unwrap().metadata.id,
        queued.metadata.id
    );
    f.provider.mode.store(1, Ordering::SeqCst);
    let submitted = f
        .application
        .execute_send_local(&f.account, &queued.metadata.id)
        .unwrap();
    assert_eq!(submitted.state, SendState::Unknown);
    f.provider.context.lock().unwrap().epoch = "changed-after-submission".into();
    assert_eq!(
        create("自动化测试：客户端一次授权").unwrap().attempt_id,
        submitted.attempt_id
    );
    assert_eq!(
        f.application
            .submit_client_send(&p, &f.account, &queued.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert_eq!(
        create("different").unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn old_credentials_do_not_upgrade_and_other_clients_cannot_approve_them() {
    let f = Fixture::new();
    let (_, old) = client(&f, false);
    let (_, bot) = client(&f, true);
    let draft = f
        .application
        .create_text_send(
            &old,
            &f.account,
            "synthetic-private-recipient-identity",
            "自动化测试：保持仅草稿",
            "owned-bot-legacy-01",
        )
        .unwrap();
    assert_eq!(draft.state, SendState::AwaitingApproval);
    assert_eq!(
        f.application
            .submit_client_send(&old, &f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "FORBIDDEN"
    );
    assert_eq!(
        f.application
            .submit_client_send(&bot, &f.account, &draft.metadata.id)
            .unwrap_err()
            .code,
        "FORBIDDEN"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn client_revocation_cancels_queued_execution_without_native_attempt() {
    let f = Fixture::new();
    let (token, p) = client(&f, true);
    let queued = f
        .application
        .create_text_send(
            &p,
            &f.account,
            "synthetic-private-recipient-identity",
            "自动化测试：撤销前未提交",
            "owned-bot-revoke-01",
        )
        .unwrap();
    f.application.revoke_token_local(&token.info.id).unwrap();
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &queued.metadata.id)
            .unwrap_err()
            .code,
        "AUTH_REVOKED"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    assert!(
        f.repository
            .send_status(&f.account, &queued.metadata.id)
            .unwrap()
            .attempt_id
            .is_none()
    );
}

#[test]
fn image_video_and_emoticon_retries_use_asset_identity_without_reencoding_or_resending() {
    let f = Fixture::new();
    let (_, p) = client(&f, true);
    let target = "synthetic-private-recipient-identity";
    let image = f
        .application
        .upload_image_local(
            &f.account,
            include_bytes!("../../../../tests/fixtures/owned-image.png").to_vec(),
        )
        .unwrap();
    let first = f
        .application
        .create_image_send(&p, &f.account, target, &image.id, "owned-bot-image-01")
        .unwrap();
    f.application
        .delete_image_local(&f.account, &image.id)
        .unwrap();
    let retry = f
        .application
        .create_image_send(&p, &f.account, target, &image.id, "owned-bot-image-01")
        .unwrap();
    assert_eq!(first.metadata.id, retry.metadata.id);
    // A removed asset stops actual execution, while an idempotent retry still returns its existing task.
    assert!(
        f.application
            .execute_send_local(&f.account, &first.metadata.id)
            .is_err()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
    let video = f
        .application
        .upload_file_local(
            &f.account,
            "owned.mp4",
            include_bytes!("../../../../tests/fixtures/owned-send-video.mp4").to_vec(),
        )
        .unwrap();
    let gif = f
        .application
        .upload_file_local(
            &f.account,
            "owned.gif",
            include_bytes!("../../../../tests/fixtures/owned-send-emoticon.gif").to_vec(),
        )
        .unwrap();
    let v = f
        .application
        .create_video_send(&p, &f.account, target, &video.id, "owned-bot-video-01")
        .unwrap();
    let g = f
        .application
        .create_emoticon_send(&p, &f.account, target, &gif.id, "owned-bot-emoticon-01")
        .unwrap();
    assert_eq!(
        f.application
            .create_video_send(&p, &f.account, target, &video.id, "owned-bot-video-01")
            .unwrap()
            .metadata
            .id,
        v.metadata.id
    );
    assert_eq!(
        f.application
            .create_emoticon_send(&p, &f.account, target, &gif.id, "owned-bot-emoticon-01")
            .unwrap()
            .metadata
            .id,
        g.metadata.id
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn finished_history_does_not_exhaust_queue_and_all_rows_remain_pageable() {
    let f = Fixture::new();
    let (_, p) = client(&f, true);
    let mut ids = std::collections::HashSet::new();
    for n in 0..515 {
        let draft = f
            .application
            .create_text_send(
                &p,
                &f.account,
                "synthetic-private-recipient-identity",
                "自动化测试：长历史",
                &format!("owned-bot-history-{n:04}"),
            )
            .unwrap();
        ids.insert(draft.metadata.id.clone());
        f.application
            .cancel_send_local(&f.account, &draft.metadata.id)
            .unwrap();
    }
    f.provider.context.lock().unwrap().epoch = "reconnected-with-long-history".into();
    let first = f
        .application
        .create_text_send(
            &p,
            &f.account,
            "synthetic-private-recipient-identity",
            "自动化测试：长历史",
            "owned-bot-history-0000",
        )
        .unwrap();
    assert!(ids.contains(&first.metadata.id));
    assert_eq!(first.state, SendState::Canceled);
    assert_eq!(
        f.application
            .create_text_send(
                &p,
                &f.account,
                "synthetic-private-recipient-identity",
                "different",
                "owned-bot-history-0000"
            )
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    let draft = f.draft("owned-bot-history-pending");
    ids.insert(draft.metadata.id.clone());
    let mut found = std::collections::HashSet::new();
    let mut before = None;
    loop {
        let page = f
            .application
            .send_history_page_local(&f.account, before, 73)
            .unwrap();
        for row in page.data {
            assert!(found.insert(row.metadata.id));
        }
        before = page.next_cursor;
        if before.is_none() {
            break;
        }
    }
    assert_eq!(found, ids);
    assert!(
        f.application
            .sends_local(&f.account)
            .unwrap()
            .iter()
            .any(|s| s.metadata.id == draft.metadata.id)
    );
}

#[test]
fn receipt_sweep_pages_all_pending_tasks_and_queries_without_resending() {
    let f = Fixture::new();
    let (_, p) = client(&f, true);
    let mut ids = Vec::new();
    for n in 0..35 {
        let task = f
            .application
            .create_text_send(
                &p,
                &f.account,
                "synthetic-private-recipient-identity",
                "自动化测试：回执分页",
                &format!("owned-receipt-sweep-{n:04}"),
            )
            .unwrap();
        f.application
            .execute_send_local(&f.account, &task.metadata.id)
            .unwrap();
        ids.push(task.metadata.id);
    }
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 35);
    let mut cursor = None;
    let mut examined = 0;
    loop {
        let progress = f
            .application
            .reconcile_receipt_page_local(&f.account, cursor)
            .unwrap();
        assert!(progress.last_error_code.is_none());
        assert_eq!(
            progress.examined, 1,
            "a receipt sweep must yield to the sender after each query"
        );
        assert_eq!(progress.updated, progress.examined);
        examined += progress.examined;
        cursor = progress.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(examined, 35);
    assert_eq!(f.provider.queries.load(Ordering::SeqCst), 35);
    for id in ids {
        assert_eq!(
            f.application
                .reconcile_send(&p, &f.account, &id)
                .unwrap()
                .state,
            SendState::RecipientConfirmed
        );
    }
    assert_eq!(
        f.application
            .reconcile_receipt_page_local(&f.account, None)
            .unwrap()
            .examined,
        0
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 35);
    assert_eq!(f.provider.queries.load(Ordering::SeqCst), 35);
}

#[test]
fn idle_worker_expires_unsubmitted_tasks_once_and_retains_unknown() {
    let f = Fixture::new();
    let draft = f.draft("owned-idle-expiry-01");
    f.approve(&draft);
    let unknown = f.draft("owned-idle-unknown-01");
    f.approve(&unknown);
    f.provider.mode.store(1, Ordering::SeqCst);
    f.application
        .execute_send_local(&f.account, &unknown.metadata.id)
        .unwrap();
    f.database()
        .execute(
            "UPDATE send_outbox SET approval_expires_at=0 WHERE id IN (?1,?2)",
            rusqlite::params![draft.metadata.id, unknown.metadata.id],
        )
        .unwrap();
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(
        f.repository
            .send_status(&f.account, &draft.metadata.id)
            .unwrap()
            .state,
        SendState::Expired
    );
    assert_eq!(
        f.repository
            .send_status(&f.account, &unknown.metadata.id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    let count = f
        .database()
        .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert!(!f.application.run_send_once_local().unwrap());
    assert_eq!(
        count,
        f.database()
            .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn client_mentions_bind_exact_members_and_retries_survive_epoch_change() {
    use werc_application::mention_send::ClientMention;
    let f = Fixture::new();
    let (_, p) = client(&f, true);
    f.provider.context.lock().unwrap().target_id = "owned-client-group@chatroom".into();
    let mut input = ClientMention {
        username: "owned-client-group@chatroom".into(),
        members: vec!["owned-member".into()],
        text: "自动化测试：明确成员".into(),
        idempotency_key: "owned-client-mention-01".into(),
    };
    let task = f
        .application
        .create_mention_send(&p, &f.account, &input)
        .unwrap();
    assert_eq!(task.state, SendState::Queued);
    assert_eq!(task.metadata.schema_version, 7);
    f.application
        .cancel_send(&p, &f.account, &task.metadata.id)
        .unwrap();
    f.provider.context.lock().unwrap().epoch = "changed-after-cancel".into();
    assert_eq!(
        f.application
            .create_mention_send(&p, &f.account, &input)
            .unwrap()
            .metadata
            .id,
        task.metadata.id
    );
    input.members = vec!["other-member".into()];
    assert_eq!(
        f.application
            .create_mention_send(&p, &f.account, &input)
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    input.members = vec!["@all".into()];
    assert_eq!(
        f.application
            .create_mention_send(&p, &f.account, &input)
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn unsupported_build_feature_never_creates_an_unknown_attempt() {
    use werc_application::mention_send::ClientMention;
    let f = Fixture::new();
    let (_, p) = client(&f, true);
    f.provider.context.lock().unwrap().target_id = "owned-client-group@chatroom".into();
    let input = ClientMention {
        username: "owned-client-group@chatroom".into(),
        members: vec!["owned-member".into()],
        text: "owned capability test".into(),
        idempotency_key: "owned-feature-gate-0001".into(),
    };
    let queued = f
        .application
        .create_mention_send(&p, &f.account, &input)
        .unwrap();
    f.provider.mode.store(6, Ordering::SeqCst);
    assert_eq!(
        f.application
            .execute_send_local(&f.account, &queued.metadata.id)
            .unwrap_err()
            .code,
        "NATIVE_FEATURE_UNAVAILABLE"
    );
    let canceled = f
        .repository
        .send_status(&f.account, &queued.metadata.id)
        .unwrap();
    assert_eq!(canceled.state, SendState::Canceled);
    assert!(canceled.attempt_id.is_none());
    let mut next = input;
    next.idempotency_key = "owned-feature-gate-0002".into();
    assert_eq!(
        f.application
            .create_mention_send(&p, &f.account, &next)
            .unwrap_err()
            .code,
        "NATIVE_FEATURE_UNAVAILABLE"
    );
    assert_eq!(f.provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn mcp_negotiation_read_boundary_and_send_share_the_same_queue() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    let f = Fixture::new();
    let (token, _) = client(&f, true);
    let router = werc_http_api::router(f.application.clone());
    let call = |path: &str, body: Value| {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("Authorization", format!("Bearer {}", token.secret))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let init=router.clone().oneshot(call("/mcp",json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"owned","version":"1"}}}))).await.unwrap();
    let result: Value =
        serde_json::from_slice(&init.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(result["result"]["protocolVersion"], "2025-06-18");
    let list = router
        .clone()
        .oneshot(call(
            "/mcp/read",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ))
        .await
        .unwrap();
    let result: Value =
        serde_json::from_slice(&list.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        result["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["annotations"]["readOnlyHint"] == true)
    );
    let send = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send_text","arguments":{"account_id":f.account,"username":"synthetic-private-recipient-identity","text":"自动化测试：MCP自有提供方","idempotency_key":"owned-mcp-send-0001"}}});
    let denied = router
        .clone()
        .oneshot(call("/mcp/read", send.clone()))
        .await
        .unwrap();
    let denied: Value =
        serde_json::from_slice(&denied.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(denied["error"]["code"], -32602);
    assert!(f.application.sends_local(&f.account).unwrap().is_empty());
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(call("/mcp", send.clone()))
            .await
            .unwrap();
        let result: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(result["result"]["isError"], false, "{result}");
        assert_eq!(result["result"]["structuredContent"]["state"], "queued");
    }
    assert_eq!(f.application.sends_local(&f.account).unwrap().len(), 1);
    let initialized = router
        .oneshot(call(
            "/mcp",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        ))
        .await
        .unwrap();
    assert_eq!(initialized.status(), 202);
    assert!(
        initialized
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}
