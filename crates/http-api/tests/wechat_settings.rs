use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tower::ServiceExt;
use werc_application::{Application, ports::CapabilityProvider};
use werc_domain::{AppError, AppResult, capability::Capability, wechat_settings::*};
use werc_infrastructure::SqliteRepository;

#[derive(Default)]
struct NativeFixture {
    bits: Mutex<u32>,
    writes: AtomicUsize,
    unknown: AtomicBool,
    epoch: Mutex<String>,
}
impl CapabilityProvider for NativeFixture {
    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
    fn wechat_settings_read(&self, _: &str) -> AppResult<SettingsSnapshot> {
        Ok(SettingsSnapshot {
            build: NativeBuild {
                version: "owned-fixture".into(),
                architecture: "x86_64".into(),
                core_sha256: "fixture".into(),
            },
            native_epoch: self.epoch.lock().unwrap().clone(),
            values: SettingKey::ALL
                .into_iter()
                .map(|key| SettingValue {
                    key,
                    value: *self.bits.lock().unwrap() & (1 << (key.native_id() - 1)) != 0,
                })
                .collect(),
            source: "owned_fixture".into(),
        })
    }
    fn wechat_settings_apply(
        &self,
        _: &str,
        t: &SettingTarget,
        epoch: &str,
    ) -> AppResult<SettingResult> {
        if *self.epoch.lock().unwrap() != epoch {
            return Err(AppError::new(
                "NATIVE_CONTEXT_CHANGED",
                "fixture connection changed",
            ));
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        let mut b = self.bits.lock().unwrap();
        let bit = 1 << (t.key.native_id() - 1);
        let before = *b & bit != 0;
        let status = if t.expected_value.is_some_and(|v| v != before) {
            ChangeStatus::Conflict
        } else if before == t.value {
            ChangeStatus::Unchanged
        } else {
            if t.value {
                *b |= bit
            } else {
                *b &= !bit
            };
            ChangeStatus::Verified
        };
        if self.unknown.load(Ordering::SeqCst) {
            return Err(AppError::new(
                "NATIVE_RESULT_UNKNOWN",
                "fixture lost response after native write",
            ));
        }
        Ok(SettingResult {
            status,
            before: Some(before),
            requested: t.value,
            observed: Some(*b & bit != 0),
            error_code: None,
        })
    }
}
fn setup(
    provider: Arc<dyn CapabilityProvider>,
    scopes: &[&str],
) -> (Arc<Application>, String, String) {
    let app = Arc::new(Application::new(
        Arc::new(SqliteRepository::in_memory().unwrap()),
        provider,
    ));
    let a = app.register_account_local("设置自有测试").unwrap();
    let t = app
        .issue_token_local(
            "fixture",
            vec![a.id.clone()],
            scopes.iter().map(|v| (*v).into()).collect(),
            1,
        )
        .unwrap();
    app.enable_api(true);
    (app, a.id, t.secret)
}
const READ: &str = "wechat:settings:read";
const WRITE: &str = "wechat:settings:write";
async fn request(
    app: &Arc<Application>,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = werc_http_api::router(app.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Authorization", format!("Bearer {token}"))
                .header("Content-Type", "application/json")
                .body(if method == "GET" {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)})),
    )
}
fn target(value: bool, expected: Option<bool>) -> Value {
    json!({"key":"new_message_sound","value":value,"expected_value":expected})
}
fn apply_body(account: &str, value: bool, key: &str) -> Value {
    json!({"account_id":account,"target":target(value,None),"idempotency_key":key})
}
#[tokio::test]
async fn http_settings_read_plan_apply_history_conflict_and_restore() {
    let native = Arc::new(NativeFixture::default());
    let (app, a, t) = setup(native.clone(), &[READ, WRITE]);
    let base = "/api/v1/wechat/settings";
    let (s, v) = request(
        &app,
        &t,
        "GET",
        &format!("{base}/capabilities?account_id={a}"),
        Value::Null,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 3);
    assert!(
        v["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["implementation"] == "native" && i["requires_foreground"] == false)
    );
    let (s, v) = request(
        &app,
        &t,
        "POST",
        &format!("{base}/plan"),
        json!({"account_id":a,"target":target(true,Some(false))}),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(v["data"]["action"], "set");
    assert_eq!(native.writes.load(Ordering::SeqCst), 0);
    let body = apply_body(&a, true, "settings-fixture-change-01");
    let (s, v) = request(&app, &t, "POST", &format!("{base}/apply"), body.clone()).await;
    assert_eq!(s, 200);
    assert_eq!(v["data"]["result"]["status"], "verified");
    let id = v["data"]["id"].as_str().unwrap().to_owned();
    let (_, v) = request(&app, &t, "POST", &format!("{base}/apply"), body).await;
    assert_eq!(v["data"]["replayed"], true);
    assert_eq!(native.writes.load(Ordering::SeqCst), 1);
    let (s, _) = request(
        &app,
        &t,
        "POST",
        &format!("{base}/apply"),
        apply_body(&a, false, "settings-fixture-change-01"),
    )
    .await;
    assert_eq!(s, 409);
    let (_, v) = request(
        &app,
        &t,
        "POST",
        &format!("{base}/apply"),
        apply_body(&a, true, "settings-fixture-same-002"),
    )
    .await;
    assert_eq!(v["data"]["result"]["status"], "unchanged");
    let (_,v)=request(&app,&t,"POST",&format!("{base}/apply"),json!({"account_id":a,"target":target(false,Some(false)),"idempotency_key":"settings-fixture-conflict"})).await;
    assert_eq!(v["data"]["result"]["status"], "conflict");
    assert_eq!(v["data"]["result"]["observed"], true);
    let (_, v) = request(
        &app,
        &t,
        "GET",
        &format!("{base}/changes?account_id={a}&limit=1"),
        Value::Null,
    )
    .await;
    let next = v["data"]["next_cursor"].as_i64().unwrap();
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 1);
    let (_, v) = request(
        &app,
        &t,
        "GET",
        &format!("{base}/changes?account_id={a}&before={next}&limit=10"),
        Value::Null,
    )
    .await;
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 2);
    let (_, v) = request(
        &app,
        &t,
        "POST",
        &format!("{base}/changes/{id}/restore"),
        json!({"account_id":a,"idempotency_key":"settings-fixture-restore1"}),
    )
    .await;
    assert_eq!(v["data"]["result"]["status"], "verified");
    assert_eq!(v["data"]["result"]["observed"], false);
    let (_, v) = request(
        &app,
        &t,
        "POST",
        &format!("{base}/read"),
        json!({"account_id":a}),
    )
    .await;
    assert_eq!(v["data"]["values"][0]["value"], false);
}
async fn tool(app: &Arc<Application>, token: &str, route: &str, name: &str, args: Value) -> Value {
    request(app,token,"POST",route,json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}})).await.1
}
#[tokio::test]
async fn mcp_tools_and_generic_read_write_enforce_the_same_permissions() {
    let native = Arc::new(NativeFixture::default());
    let (app, a, t) = setup(native.clone(), &[READ, WRITE]);
    let v = tool(
        &app,
        &t,
        "/mcp",
        "wechat_settings_apply",
        apply_body(&a, true, "settings-mcp-native-001"),
    )
    .await;
    assert_eq!(
        v["result"]["structuredContent"]["data"]["result"]["status"],
        "verified"
    );
    let v = tool(
        &app,
        &t,
        "/mcp/read",
        "wechat_settings_read",
        json!({"account_id":a}),
    )
    .await;
    assert_eq!(
        v["result"]["structuredContent"]["data"]["values"][0]["value"],
        true
    );
    let v = tool(
        &app,
        &t,
        "/mcp/read",
        "wechat_settings_apply",
        apply_body(&a, false, "settings-mcp-blocked-01"),
    )
    .await;
    assert!(v.get("error").is_some() || v["result"]["isError"] == true);
    for (route, name) in [("/mcp", "api_read"), ("/mcp/read", "api_call")] {
        let v=tool(&app,&t,route,name,json!({"method":"POST","path":"/api/v1/wechat/settings/apply","body":apply_body(&a,false,"settings-mcp-blocked-02")})).await;
        assert!(v.get("error").is_some() || v["result"]["isError"] == true);
    }
    let v=tool(&app,&t,"/mcp/read","api_read",json!({"method":"POST","path":"/api/v1/wechat/settings/plan","body":{"account_id":a,"target":target(false,None)}})).await;
    assert_eq!(v["result"]["structuredContent"]["status"], 200);
    let (s, v) = request(
        &app,
        &t,
        "POST",
        "/mcp/read",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )
    .await;
    assert_eq!(s, 200);
    let tools = v["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|v| v["name"] == "wechat_settings_read"));
    assert!(!tools.iter().any(|v| v["name"] == "wechat_settings_apply"));
    assert_eq!(native.writes.load(Ordering::SeqCst), 1);
    let read = app
        .issue_token_local("read", vec![a.clone()], vec![READ.into()], 1)
        .unwrap();
    let (s, _) = request(
        &app,
        &read.secret,
        "POST",
        "/api/v1/wechat/settings/apply",
        apply_body(&a, false, "settings-read-blocked-01"),
    )
    .await;
    assert_eq!(s, 403);
    let (s, _) = request(
        &app,
        &t,
        "POST",
        "/api/v1/wechat/settings/read",
        json!({"account_id":"other"}),
    )
    .await;
    assert_eq!(s, 403);
}
#[tokio::test]
async fn unknown_results_survive_restart_and_never_resubmit() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/settings-native-research/stores");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.sqlite3", uuid::Uuid::new_v4()));
    let native = Arc::new(NativeFixture::default());
    native.unknown.store(true, Ordering::SeqCst);
    let app = Arc::new(Application::new(
        Arc::new(SqliteRepository::open(&path).unwrap()),
        native.clone(),
    ));
    let a = app.register_account_local("owned-persistence").unwrap();
    let t = app
        .issue_token_local(
            "fixture",
            vec![a.id.clone()],
            vec![READ.into(), WRITE.into()],
            1,
        )
        .unwrap();
    app.enable_api(true);
    let body = apply_body(&a.id, true, "settings-unknown-durable1");
    let (_, v) = request(
        &app,
        &t.secret,
        "POST",
        "/api/v1/wechat/settings/apply",
        body.clone(),
    )
    .await;
    assert_eq!(v["data"]["result"]["status"], "unknown");
    assert_eq!(native.writes.load(Ordering::SeqCst), 1);
    drop(app);
    native.unknown.store(false, Ordering::SeqCst);
    let app = Arc::new(Application::new(
        Arc::new(SqliteRepository::open(&path).unwrap()),
        native.clone(),
    ));
    app.enable_api(true);
    let (_, v) = request(
        &app,
        &t.secret,
        "POST",
        "/api/v1/wechat/settings/apply",
        body,
    )
    .await;
    assert_eq!(v["data"]["replayed"], true);
    assert_eq!(v["data"]["result"]["status"], "unknown");
    assert_eq!(native.writes.load(Ordering::SeqCst), 1);
    let (_, v) = request(
        &app,
        &t.secret,
        "POST",
        "/api/v1/wechat/settings/read",
        json!({"account_id":a.id}),
    )
    .await;
    assert_eq!(v["data"]["values"][0]["value"], true);
}
#[tokio::test]
async fn restore_rejects_changed_connection_and_preserves_manual_changes() {
    let native = Arc::new(NativeFixture::default());
    let (app, a, t) = setup(native.clone(), &[READ, WRITE]);
    let (_, v) = request(
        &app,
        &t,
        "POST",
        "/api/v1/wechat/settings/apply",
        apply_body(&a, true, "settings-restore-base01"),
    )
    .await;
    let id = v["data"]["id"].as_str().unwrap();
    *native.epoch.lock().unwrap() = "new-login".into();
    let (s, _) = request(
        &app,
        &t,
        "POST",
        &format!("/api/v1/wechat/settings/changes/{id}/restore"),
        json!({"account_id":a,"idempotency_key":"settings-restore-epoch1"}),
    )
    .await;
    assert_eq!(s, 409);
    *native.epoch.lock().unwrap() = String::new();
    *native.bits.lock().unwrap() = 0;
    let (_, v) = request(
        &app,
        &t,
        "POST",
        &format!("/api/v1/wechat/settings/changes/{id}/restore"),
        json!({"account_id":a,"idempotency_key":"settings-restore-user01"}),
    )
    .await;
    assert_eq!(v["data"]["result"]["status"], "conflict");
}
#[tokio::test]
async fn unsupported_providers_offer_no_settings_or_foreground_fallback() {
    let (app, a, t) = setup(
        Arc::new(werc_wechat_win::WindowsWechatProvider),
        &[READ, WRITE],
    );
    let (s, v) = request(
        &app,
        &t,
        "GET",
        &format!("/api/v1/wechat/settings/capabilities?account_id={a}"),
        Value::Null,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(v["data"]["available"], false);
    assert_eq!(v["data"]["items"], json!([]));
    let (s, _) = request(
        &app,
        &t,
        "POST",
        "/api/v1/wechat/settings/apply",
        apply_body(&a, true, "settings-unsupported-01"),
    )
    .await;
    assert_eq!(s, 501);
    for bad in [
        json!({"key":"unknown","value":true}),
        json!({"key":"new_message_sound","value":"yes"}),
        json!({"key":"new_message_sound","value":true,"address":123}),
    ] {
        let (s, _) = request(
            &app,
            &t,
            "POST",
            "/api/v1/wechat/settings/plan",
            json!({"account_id":a,"target":bad}),
        )
        .await;
        assert_eq!(s, 422);
    }
}
