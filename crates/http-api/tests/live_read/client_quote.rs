use super::*;
use werc_application::{
    conversation_media::ConversationMediaSelection, live_conversation::ConversationCommand as C,
    quote_send::ClientQuote, recipients::RecipientSelection,
};

#[test]
fn client_quote_authenticates_source_queues_once_and_retries_after_view_clear() {
    let f = Fixture::new();
    let parent = f.prepare().unwrap();
    let target = "owned-bot-quote";
    let page = f
        .app
        .live_conversation_local(
            &f.account,
            C::Open {
                parent_view_id: parent.view_id,
                username: target.into(),
            },
            "owned-bot-quote-open",
        )
        .unwrap()
        .unwrap();
    f.app
        .prepare_recipient_local(
            &f.account,
            RecipientSelection {
                view_id: page.messages.view_id.clone(),
                multi: false,
            },
        )
        .unwrap();
    let token = f
        .app
        .issue_token_local(
            "自动化测试引用客户端",
            vec![f.account.clone()],
            [
                "messages:send",
                "messages:read",
                "messages:execute",
                "operations:create",
                "operations:read",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let principal = f.app.authenticate(&token.secret).unwrap();
    let mut input = ClientQuote {
        username: target.into(),
        server_id: page.messages.rows[0].server_id.clone(),
        position: None,
        selection: Some(ConversationMediaSelection {
            view_id: page.messages.view_id.clone(),
            local_id: page.messages.rows[0].local_id.clone(),
            source: None,
        }),
        text: "自动化测试：引用接入".into(),
        idempotency_key: "owned-bot-quote-task".into(),
    };
    let server = input.server_id.clone();
    input.server_id = Some("123".into());
    assert_eq!(
        f.app
            .create_quote_send(&principal, &f.account, &input)
            .unwrap_err()
            .code,
        "QUOTE_SOURCE_INVALID"
    );
    input.server_id = server;
    let draft = f
        .app
        .create_quote_send(&principal, &f.account, &input)
        .unwrap();
    assert_eq!(draft.state, SendState::Queued);
    assert_eq!(draft.metadata.creator_id, principal.id);
    let submitted = f
        .app
        .execute_send_local(&f.account, &draft.metadata.id)
        .unwrap();
    assert_eq!(submitted.state, SendState::Unknown);
    // Losing a source view does not cause another submission of the same authenticated quote.
    input.selection.as_mut().unwrap().view_id = "expired-view".into();
    f.reader.context.lock().unwrap().epoch = "new-connection".into();
    assert_eq!(
        f.app
            .create_quote_send(&principal, &f.account, &input)
            .unwrap()
            .attempt_id,
        submitted.attempt_id
    );
    input.server_id = Some("456".into());
    assert_eq!(
        f.app
            .create_quote_send(&principal, &f.account, &input)
            .unwrap_err()
            .code,
        "IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(f.reader.quote_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn native_position_quotes_exact_inbound_private_and_group_text() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use werc_domain::receive::{ReceivedRoute, ReceivedText};

    let f = Fixture::new();
    let private = "owned-friend";
    let group = "owned-group@chatroom";
    *f.reader.recipients.lock().unwrap() = vec![private.into(), group.into()];
    let token = f
        .app
        .issue_token_local(
            "quote position",
            vec![f.account.clone()],
            [
                "messages:send",
                "messages:read",
                "messages:execute",
                "operations:create",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            1,
        )
        .unwrap();
    f.app.enable_api(true);
    let router = werc_http_api::router(f.app.clone());
    let request = |account: &str, message: Value| {
        let req = Request::post("/api/v1/messages/send/quote")
            .header("authorization", format!("Bearer {}", token.secret))
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"account_id":account,"message":message}).to_string(),
            ))
            .unwrap();
        let router = router.clone();
        async move {
            let response = router.oneshot(req).await.unwrap();
            let status = response.status();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            (status, serde_json::from_slice::<Value>(&body).unwrap())
        }
    };
    let observe = |sequence, server_id: &str, target: &str, outgoing, message_type| {
        *f.reader.quote_observation.lock().unwrap() = Some((
            ReceivedRoute {
                sender: if outgoing { "owned" } else { target }.into(),
                receiver: if outgoing { target } else { "owned" }.into(),
                conversation: target.into(),
            },
            ReceivedText {
                sequence,
                native_message_id: server_id.into(),
                message_type,
                text: if message_type != 1 {
                    String::new()
                } else if target == group {
                    "group-member:\nquoted-literal:\nexact group text".into()
                } else {
                    "quoted-literal:\nexact private text".into()
                },
                metadata: (message_type != 1).then(|| "<owned/>".into()),
            },
        ));
        f.app.receive_once_local(&f.account).unwrap();
        f.app
            .received_texts_local(&f.account, -1, 100)
            .unwrap()
            .last()
            .unwrap()
            .position
    };
    let private_pos = observe(1, "9007199254741997", private, false, 1);
    let identical_newer = observe(2, "9007199254741998", private, false, 1);
    let group_pos = observe(3, "9007199254741999", group, false, 1);
    let zero_id = observe(4, "0", group, false, 1);
    let media = observe(5, "9007199254742001", group, false, 3);
    let outgoing = observe(6, "9007199254742002", group, true, 1);
    let msg = |target: &str, position, key: &str| json!({"username":target,"position":position,"text":"owned reply","idempotency_key":key});
    let key = "owned-observed-quote-private";
    let (status, response) = request(&f.account, msg(private, private_pos, key)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let private_id = response["data"]["metadata"]["id"].as_str().unwrap();
    let (_, repeated) = request(&f.account, msg(private, private_pos, key)).await;
    assert_eq!(repeated["data"]["metadata"]["id"], private_id);
    let (_, conflict) = request(&f.account, msg(private, identical_newer, key)).await;
    assert_eq!(
        conflict["error"]["code"], "IDEMPOTENCY_CONFLICT",
        "{conflict}"
    );
    let mut group_request = msg(group, group_pos, "owned-observed-quote-group");
    group_request["server_id"] = json!("9007199254741999");
    // Keep the sender inside its native call, with the send lock held. An
    // independently authenticated HTTP draft must still be accepted.
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    *f.reader.quote_submit_pause.lock().unwrap() = Some((entered_tx, resume_rx));
    let app = f.app.clone();
    let account = f.account.clone();
    let private_to_send = private_id.to_owned();
    let sender = std::thread::spawn(move || app.execute_send_local(&account, &private_to_send));
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let (status, response) = request(&f.account, group_request.clone()).await;
    resume_tx.send(()).unwrap();
    assert_eq!(sender.join().unwrap().unwrap().state, SendState::Unknown);
    assert_eq!(status, StatusCode::OK, "{response}");
    let group_id = response["data"]["metadata"]["id"].as_str().unwrap();
    for (id, body) in [
        (private_id, "quoted-literal:\nexact private text"),
        (group_id, "quoted-literal:\nexact group text"),
    ] {
        let preview = f.app.send_preview_local(&f.account, id).unwrap();
        assert_eq!(preview.payload.quote.as_ref().unwrap().text, body);
    }
    for (target, position) in [
        (private, group_pos),
        (group, zero_id),
        (group, media),
        (group, outgoing),
        (group, outgoing + 1),
    ] {
        let (_, invalid) = request(
            &f.account,
            msg(target, position, "owned-observed-quote-invalid"),
        )
        .await;
        assert_eq!(
            invalid["error"]["code"], "QUOTE_SOURCE_INVALID",
            "{invalid}"
        );
    }
    group_request["selection"] = json!({"view_id":"unused","local_id":"unused"});
    let (_, both) = request(&f.account, group_request).await;
    assert_eq!(both["error"]["code"], "INVALID_ARGUMENT", "{both}");
    let mut incorrect_id = msg(group, group_pos, "owned-observed-quote-invalid");
    incorrect_id["server_id"] = json!("9007199254741998");
    let (_, invalid) = request(&f.account, incorrect_id).await;
    assert_eq!(invalid["error"]["code"], "QUOTE_SOURCE_INVALID");
    let (status, _) = request(
        "wrong-account",
        msg(group, group_pos, "owned-observed-quote-invalid"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        f.app
            .execute_send_local(&f.account, group_id)
            .unwrap()
            .state,
        SendState::Unknown
    );
    assert_eq!(
        *f.reader.quote_source_ids.lock().unwrap(),
        ["9007199254741997", "9007199254741999"]
    );
    f.reader.context.lock().unwrap().sender_id = "different-login".into();
    let (_, invalid) = request(
        &f.account,
        msg(group, group_pos, "owned-observed-quote-invalid"),
    )
    .await;
    assert_eq!(
        invalid["error"]["code"], "QUOTE_SOURCE_INVALID",
        "{invalid}"
    );
}
