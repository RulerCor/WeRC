//! Explicit acceptance on the bot-owned login. Never part of normal cargo test.
#![cfg(windows)]
use serde_json::{Value, json};
use std::sync::Arc;
use werc_application::Application;
use werc_domain::{Principal, wechat_settings::*};
use werc_infrastructure::SqliteRepository;
use werc_wechat_win::{WindowsWechatProvider, native_messages::NativeFileHelperProvider};

struct Restore {
    app: Arc<Application>,
    principal: Principal,
    account: String,
    values: Vec<SettingValue>,
}
impl Drop for Restore {
    fn drop(&mut self) {
        for v in &self.values {
            let r = self.app.wechat_settings_apply(
                &self.principal,
                &self.account,
                &SettingTarget {
                    key: v.key,
                    value: v.value,
                    expected_value: None,
                },
                &format!("live-cleanup-{}", uuid::Uuid::new_v4()),
            );
            assert!(
                r.is_ok_and(|r| matches!(
                    r.result.status,
                    ChangeStatus::Verified | ChangeStatus::Unchanged
                )),
                "native cleanup must restore original preference"
            );
        }
    }
}
async fn http(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> Value {
    let request = if method == "GET" {
        client.get(format!("{url}{path}"))
    } else {
        client
            .post(format!("{url}{path}"))
            .header("Content-Type", "application/json")
            .body(body.to_string())
    };
    let response = request.bearer_auth(token).send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "HTTP {status}: {text}");
    serde_json::from_str(&text).unwrap()
}
async fn mcp(client: &reqwest::Client, url: &str, token: &str, name: &str, args: Value) -> Value {
    let v=http(client,url,token,"POST","/mcp",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).await;
    assert_eq!(v["result"]["isError"], false, "MCP: {v}");
    v["result"]["structuredContent"]["data"].clone()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit bot-owned current WeChat PID, exact source-built host and reversible preferences acceptance"]
async fn background_native_settings_through_real_tcp_http_and_mcp() {
    let pid = std::env::var("WERC_SETTINGS_TEST_PID")
        .unwrap()
        .parse()
        .unwrap();
    let host = std::path::PathBuf::from(std::env::var("WERC_SETTINGS_HOST").unwrap());
    let hash = std::env::var("WERC_SETTINGS_HOST_SHA256").unwrap();
    let exe = std::env::var("WERC_SETTINGS_WECHAT_EXE").unwrap();
    let repo = Arc::new(SqliteRepository::in_memory().unwrap());
    let registration = Application::new(repo.clone(), Arc::new(WindowsWechatProvider));
    let account = registration
        .register_account_local("原生设置隔离验收")
        .unwrap();
    let native = NativeFileHelperProvider::connect(&account.id, &host, &hash, pid, &exe).unwrap();
    let app = Arc::new(Application::new(repo, Arc::new(native)));
    let token = app
        .issue_token_local(
            "native settings acceptance",
            vec![account.id.clone()],
            vec![
                "wechat:settings:read".into(),
                "wechat:settings:write".into(),
            ],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let principal = app.authenticate(&token.secret).unwrap();
    let initial = app.wechat_settings_read(&principal, &account.id).unwrap();
    assert_eq!(initial.build.version, std::env::var("WERC_SETTINGS_EXPECT_VERSION").unwrap_or_else(|_|"4.1.15.13".into()));
    assert_eq!(initial.values.len(), 3);
    let restore = Restore {
        app: app.clone(),
        principal,
        account: account.id.clone(),
        values: initial.values.clone(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let routes = werc_http_api::router(app.clone());
    let server = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
    let client = reqwest::Client::new();
    let cap = http(
        &client,
        &url,
        &token.secret,
        "GET",
        &format!(
            "/api/v1/wechat/settings/capabilities?account_id={}",
            account.id
        ),
        Value::Null,
    )
    .await;
    assert_eq!(cap["data"]["items"].as_array().unwrap().len(), 3);
    assert!(
        cap["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["requires_foreground"] == false && i["implementation"] == "native")
    );
    let mut checks =
        vec!["exact native core / live capabilities / no foreground requirement".to_owned()];
    for v in &initial.values {
        let body = json!({"account_id":account.id,"target":{"key":v.key,"value":!v.value,"expected_value":v.value},"idempotency_key":format!("live-http-{}",uuid::Uuid::new_v4())});
        let r = http(
            &client,
            &url,
            &token.secret,
            "POST",
            "/api/v1/wechat/settings/apply",
            body.clone(),
        )
        .await;
        assert_eq!(r["data"]["result"]["status"], "verified");
        assert_eq!(r["data"]["result"]["observed"], !v.value);
        let id = r["data"]["id"].clone();
        let repeated = http(
            &client,
            &url,
            &token.secret,
            "POST",
            "/api/v1/wechat/settings/apply",
            body,
        )
        .await;
        assert_eq!(repeated["data"]["replayed"], true);
        let read = mcp(
            &client,
            &url,
            &token.secret,
            "wechat_settings_read",
            json!({"account_id":account.id}),
        )
        .await;
        let key = serde_json::to_value(v.key).unwrap();
        assert_eq!(
            read["values"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["key"] == key)
                .unwrap()["value"],
            !v.value
        );
        let same=mcp(&client,&url,&token.secret,"wechat_settings_apply",json!({"account_id":account.id,"target":{"key":v.key,"value":!v.value},"idempotency_key":format!("live-mcp-same-{}",uuid::Uuid::new_v4())})).await;
        assert_eq!(same["result"]["status"], "unchanged");
        let restored=mcp(&client,&url,&token.secret,"wechat_settings_restore",json!({"account_id":account.id,"id":id,"idempotency_key":format!("live-mcp-restore-{}",uuid::Uuid::new_v4())})).await;
        assert_eq!(restored["result"]["status"], "verified");
        assert_eq!(restored["result"]["observed"], v.value);
        checks.push(format!(
            "{:?}: HTTP apply / replay / MCP native read / unchanged / restore",
            v.key
        ));
    }
    let final_state = app
        .wechat_settings_read(&restore.principal, &account.id)
        .unwrap();
    assert!(
        initial
            .values
            .iter()
            .zip(&final_state.values)
            .all(|(a, b)| a.key == b.key && a.value == b.value)
    );
    let report = json!({"schema_version":1,"transport":"loopback TCP HTTP and MCP","core":initial.build,"checks":checks,"all_original_values_restored":true,"messages_sent":false,"foreground_operations":false});
    let out = std::env::var_os("WERC_SETTINGS_RESULT").map(std::path::PathBuf::from).unwrap_or_else(||std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/settings-native-research/live-api-result.json"));
    std::fs::write(out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    server.abort();
    drop(restore);
    drop(client);
    drop(app);
}
