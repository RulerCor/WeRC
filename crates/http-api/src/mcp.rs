//! Stateless Streamable HTTP MCP. Business policy belongs to the client.
use super::*;
use serde_json::Value;
const VERSION: &str = "2025-06-18";

struct Tool {
    name: &'static str,
    description: &'static str,
    scopes: &'static [&'static str],
    write: bool,
    schema: Value,
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn tools() -> Vec<Tool> {
    let account = || object(json!({"account_id":{"type":"string"}}), &["account_id"]);
    let status = || {
        object(
            json!({"account_id":{"type":"string"},"outbox_id":{"type":"string"}}),
            &["account_id", "outbox_id"],
        )
    };
    let mut items = vec![
        Tool {
            name: "list_accounts",
            description: "当前令牌可访问的账号",
            scopes: &["accounts:read"],
            write: false,
            schema: object(json!({}), &[]),
        },
        Tool {
            name: "get_capabilities",
            description: "原生能力及当前可用状态",
            scopes: &["capabilities:read"],
            write: false,
            schema: object(json!({}), &[]),
        },
        Tool {
            name: "get_connection",
            description: "账号的客户端连接信息",
            scopes: &["connections:read"],
            write: false,
            schema: account(),
        },
        Tool {
            name: "get_messages",
            description: "读取持久化原生消息；不按会话或免打扰过滤。保存next_cursor用于续读",
            scopes: &["messages:read"],
            write: false,
            schema: object(
                json!({"account_id":{"type":"string"},"after":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":100,"default":100}}),
                &["account_id"],
            ),
        },
        Tool {
            name: "get_events",
            description: "读取平台事件和发送结果变化，游标与消息位置分别保存",
            scopes: &["events:read"],
            write: false,
            schema: object(
                json!({"account_id":{"type":"string"},"after":{"type":"integer","minimum":0,"default":0},"limit":{"type":"integer","minimum":1,"maximum":200,"default":100}}),
                &["account_id"],
            ),
        },
        Tool {
            name: "list_sends",
            description: "分页查询全部持久发送任务；next_cursor用于下一次before，Unknown不重发",
            scopes: &["operations:read"],
            write: false,
            schema: object(
                json!({"account_id":{"type":"string"},"before":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":200,"default":100}}),
                &["account_id"],
            ),
        },
        Tool {
            name: "get_send_status",
            description: "查询真实提交与回执状态；受理不表示接收端送达",
            scopes: &["operations:read"],
            write: false,
            schema: status(),
        },
        Tool {
            name: "submit_send",
            description: "提交当前客户端自己的草稿；重复调用返回原状态，不重复发送",
            scopes: &["operations:create", "messages:execute"],
            write: true,
            schema: status(),
        },
        Tool {
            name: "reconcile_send",
            description: "回查已有提交的原生回执并更新持久状态；不重新发送。旧连接身份的结果可能仍为Unknown",
            scopes: &["operations:read"],
            write: false,
            schema: status(),
        },
        Tool {
            name: "cancel_send",
            description: "取消当前客户端尚未提交的发送任务",
            scopes: &["operations:create"],
            write: true,
            schema: status(),
        },
        Tool {
            name: "prepare_recipient",
            description: "用已认证聊天窗口准备精确收件人，无需桌面点击；本身不发送",
            scopes: &["messages:execute"],
            write: true,
            schema: object(
                json!({"account_id":{"type":"string"},"view_id":{"type":"string"},"multi":{"type":"boolean"}}),
                &["account_id", "view_id", "multi"],
            ),
        },
    ];
    let selection = object(
        json!({"view_id":{"type":"string"},"local_id":{"type":"string"},"source":{"type":["string","null"]}}),
        &["view_id", "local_id"],
    );
    let mut mention = json!({"username":{"type":"string"},"members":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":8,"uniqueItems":true},"text":{"type":"string"},"idempotency_key":{"type":"string","minLength":16,"maxLength":128}});
    items.push(Tool {
        name: "send_mention", description: "原生群成员提醒；使用精确群账号和1至8个不同成员账号，Bot负责选择成员，不支持昵称或@所有人",
        scopes: &["messages:send", "operations:create", "messages:execute"], write: true,
        schema: object(json!({"account_id":{"type":"string"},"message":object(mention.clone(), &["username","members","text","idempotency_key"])}), &["account_id","message"]),
    });
    mention.as_object_mut().unwrap().remove("members");
    mention["server_id"] = json!({"type":"string","description":"原消息完整服务器标识；selection 模式必填，position 模式可选核对"});
    mention["selection"] = selection;
    mention["position"] = json!({"type":"integer","minimum":1,"description":"同一账号原生来信的 SSE 事件位置"});
    let mut quote = object(mention, &["username", "text", "idempotency_key"]);
    quote["oneOf"] = json!([
        {"required":["position"],"not":{"required":["selection"]}},
        {"required":["selection","server_id"],"not":{"required":["position"]}}
    ]);
    items.push(Tool {
        name: "send_quote", description: "原生引用普通文字：使用实时来信 position 或会话视图 selection 精确定位",
        scopes: &["messages:send", "messages:read", "operations:create", "messages:execute"], write: true,
        schema: object(json!({"account_id":{"type":"string"},"message":quote}), &["account_id","message"]),
    });
    for (name, scope, text) in [
        ("send_text", "messages:send", true),
        ("send_voice", "messages:send_voice", false),
        ("send_image", "messages:send_image", false),
        ("send_file", "messages:send_file", false),
        ("send_video", "messages:send_video", false),
        ("send_emoticon", "messages:send_emoticon", false),
    ] {
        let mut properties = json!({"account_id":{"type":"string"},"username":{"type":"string"},"idempotency_key":{"type":"string","minLength":16,"maxLength":128}});
        let field = if text { "text" } else { "asset_id" };
        properties[field] = json!({"type":"string"});
        // Each distinct call must use its own key; retries keep the same key.
        let scopes: &'static [&'static str] = match scope {
            "messages:send" => &["messages:send", "operations:create", "messages:execute"],
            "messages:send_voice" => &[
                "messages:send_voice",
                "operations:create",
                "messages:execute",
            ],
            "messages:send_image" => &[
                "messages:send_image",
                "operations:create",
                "messages:execute",
            ],
            "messages:send_file" => &[
                "messages:send_file",
                "operations:create",
                "messages:execute",
            ],
            "messages:send_video" => &[
                "messages:send_video",
                "operations:create",
                "messages:execute",
            ],
            _ => &[
                "messages:send_emoticon",
                "operations:create",
                "messages:execute",
            ],
        };
        items.push(Tool {
            name,
            description: if text {
                "原生发送文字；重试复用同一幂等键。直接排队执行，不需要桌面再批准"
            } else {
                "发送已通过HTTP上传的媒体asset_id；重试复用同一幂等键，直接排队执行"
            },
            scopes,
            write: true,
            schema: object(
                properties,
                &["account_id", "username", "idempotency_key", field],
            ),
        });
    }
    for (name, write) in [("api_read", false), ("api_call", true)] {
        items.push(Tool{name,write,scopes:&[],description:if write {"调用完整本机HTTP能力。先用api_read读取/api/v1/openapi.json获取参数与媒体上传格式；每个接口仍独立校验账号和权限。multipart等原始请求使用body_hex和content_type；不要在URL中放令牌。不会自动重试外发。"} else {"访问只读HTTP能力，包括在线聊天、搜索、联系人、群成员、朋友圈、媒体和归档。GET或列明的只读POST；不可执行发送、删除或上传。先GET /api/v1/openapi.json查看接口。"},schema:object(json!({"method":{"type":"string","enum":if write {vec!["GET","POST","DELETE"]}else{vec!["GET","POST"]}},"path":{"type":"string","description":"以/api/v1/开头的路径，可含查询参数；不支持无限SSE流"},"body":{"type":"object"},"body_hex":{"type":"string"},"content_type":{"type":"string"},"idempotency_key":{"type":"string","minLength":16,"maxLength":128}}),&["method","path"])});
    }
    let target = object(json!({"key":{"type":"string","enum":["new_message_sound","call_sound","auto_download"]},"value":{"type":"boolean"},"expected_value":{"type":"boolean"}}), &["key","value"]);
    for (name,description,write,properties,required) in [
        ("wechat_settings_capabilities","当前精确构建已验证的后台原生设置；不采用前台操作",false,json!({"account_id":{"type":"string"}}),vec!["account_id"]),
        ("wechat_settings_read","从微信原生设置来源读取当前值",false,json!({"account_id":{"type":"string"}}),vec!["account_id"]),
        ("wechat_settings_plan","预览一个设置目标的差异；不修改微信",false,json!({"account_id":{"type":"string"},"target":target.clone()}),vec!["account_id","target"]),
        ("wechat_settings_apply","后台原生设置并复读；unknown 先读取，不重新提交；幂等键持久保存",true,json!({"account_id":{"type":"string"},"target":target,"idempotency_key":{"type":"string","minLength":16,"maxLength":128}}),vec!["account_id","target","idempotency_key"]),
        ("wechat_settings_changes","分页查看本账号设置变更记录",false,json!({"account_id":{"type":"string"},"before":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"maximum":100,"default":20}}),vec!["account_id"]),
        ("wechat_settings_restore","恢复已验证记录的原值；当前值或连接变化则拒绝覆盖",true,json!({"account_id":{"type":"string"},"id":{"type":"string"},"idempotency_key":{"type":"string","minLength":16,"maxLength":128}}),vec!["account_id","id","idempotency_key"]),
    ] { items.push(Tool{name,description,write,scopes:if write {&["wechat:settings:write"]}else{&["wechat:settings:read"]},schema:object(properties,&required)}); }
    items
}
fn allowed(tool: &Tool, p: &Principal, read_only: bool) -> bool {
    (!read_only || !tool.write)
        && tool
            .scopes
            .iter()
            .all(|scope| p.scopes.iter().any(|s| s == scope))
}
fn rpc_error(id: Value, code: i32, message: &str) -> Response {
    Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})).into_response()
}
fn rpc_result(id: Value, result: Value) -> Response {
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}
pub(super) async fn endpoint(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    handle(state, headers, body, false).await
}
pub(super) async fn read_endpoint(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    handle(state, headers, body, true).await
}
async fn handle(
    state: ApiState,
    headers: HeaderMap,
    body: axum::body::Bytes,
    read_only: bool,
) -> Response {
    if headers
        .get("mcp-protocol-version")
        .is_some_and(|v| v != VERSION && v != "2025-03-26")
    {
        return (StatusCode::BAD_REQUEST, "Unsupported MCP protocol version").into_response();
    }
    let request: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return rpc_error(Value::Null, -32700, "Parse error"),
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if !request.is_object()
        || request["jsonrpc"] != "2.0"
        || request
            .get("id")
            .is_some_and(|v| !v.is_string() && !v.is_i64() && !v.is_u64())
    {
        return rpc_error(Value::Null, -32600, "Invalid Request");
    }
    let Some(method) = request["method"].as_str() else {
        return rpc_error(id, -32600, "Invalid Request");
    };
    // Notifications carry no response and never trigger a tool side effect.
    if request.get("id").is_none() {
        return if method.starts_with("notifications/") {
            StatusCode::ACCEPTED.into_response()
        } else {
            rpc_error(Value::Null, -32600, "Request id required")
        };
    }
    let secret = match bearer(&headers) {
        Ok(s) => s.to_owned(),
        Err(e) => return e.into_response(),
    };
    let principal = match state.application.authenticate(&secret) {
        Ok(p) => p,
        Err(e) => return ApiError(e).into_response(),
    };
    match method {
        "initialize" => {
            if !request["params"]["protocolVersion"].is_string()
                || !request["params"]["capabilities"].is_object()
                || !request["params"]["clientInfo"]["name"].is_string()
                || !request["params"]["clientInfo"]["version"].is_string()
            {
                return rpc_error(id, -32602, "Invalid initialize parameters");
            }
            rpc_result(
                id,
                json!({"protocolVersion":VERSION,"capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"WeRC","version":env!("CARGO_PKG_VERSION")},
                "instructions":"业务规则由客户端负责。使用get_messages/get_events分别保存游标；发送重试保持同一idempotency_key，Unknown不可重发。"}),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => {
            let data: Vec<_> = tools().into_iter().filter(|t| allowed(t, &principal, read_only)).map(|t| {
                json!({"name":t.name,"description":t.description,"inputSchema":t.schema,
                    "annotations":{"readOnlyHint":!t.write,"destructiveHint":t.write,"openWorldHint":t.write}})
            }).collect();
            rpc_result(id, json!({"tools":data}))
        }
        "tools/call" => {
            let Some(name) = request["params"]["name"].as_str() else {
                return rpc_error(id, -32602, "Tool name required");
            };
            let Some(tool) = tools().into_iter().find(|tool| tool.name == name) else {
                return rpc_error(id, -32602, "Unknown tool");
            };
            if !allowed(&tool, &principal, read_only) {
                return rpc_error(
                    id,
                    -32602,
                    "Tool unavailable for this endpoint or credential",
                );
            }
            let arguments = request["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let name = name.to_owned();
            if matches!(name.as_str(), "api_read" | "api_call") {
                let output = api_call(state.clone(), &secret, arguments, name == "api_read").await;
                if let Err(e) = state.application.authenticate(&secret) {
                    return ApiError(e).into_response();
                }
                let (value, error) = match output {
                    Ok(value) => {
                        let failed = value["status"].as_u64().is_some_and(|s| s >= 400);
                        (value, failed)
                    }
                    Err(error) => (json!({"error":error.0}), true),
                };
                return rpc_result(
                    id,
                    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":error}),
                );
            }
            let app = state.application.clone();
            let auth = secret.clone();
            let output = local_work(app, move |app| {
                let p = app.authenticate(&auth)?;
                call_tool(app, &p, &name, arguments)
            })
            .await;
            // Returning results is also subject to a revoked/expired credential.
            if let Err(e) = state.application.authenticate(&secret) {
                return ApiError(e).into_response();
            }
            let (value, error) = match output {
                Ok(v) => (v, false),
                Err(e) => (json!({"error":e.0}), true),
            };
            rpc_result(
                id,
                json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":error}),
            )
        }
        _ => rpc_error(id, -32601, "Method not found"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiCall {
    method: String,
    path: String,
    body: Option<Value>,
    body_hex: Option<String>,
    content_type: Option<String>,
    idempotency_key: Option<String>,
}
async fn api_call(
    state: ApiState,
    secret: &str,
    args: Value,
    read_only: bool,
) -> Result<Value, ApiError> {
    use tower::ServiceExt;
    let input: ApiCall = decode(args)?;
    let Some(path) = input.path.strip_prefix("/api/v1") else {
        return Err(AppError::new("INVALID_ARGUMENT", "只接受/api/v1本机路径").into());
    };
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains('#')
        || path.len() > 8192
        || path
            .split('?')
            .next()
            .is_some_and(|p| p.ends_with("/stream"))
        || !matches!(input.method.as_str(), "GET" | "POST" | "DELETE")
        || input.body.is_some() && input.body_hex.is_some()
    {
        return Err(AppError::new("INVALID_ARGUMENT", "HTTP工具路径、方法或正文无效").into());
    }
    let route = path.split('?').next().unwrap_or(path);
    let read_post = matches!(
        route,
        "/browse/conversations"
            | "/browse/messages"
            | "/browse/cancel"
            | "/contacts/live"
            | "/contacts/live/group"
            | "/contacts/live/avatar"
            | "/contacts/live/cancel"
            | "/contacts/live/group/cancel"
            | "/sessions/live"
            | "/sessions/live/cancel"
            | "/records/live"
            | "/records/live/export"
            | "/records/live/cancel"
            | "/favorites/live"
            | "/favorites/live/media"
            | "/favorites/live/cancel"
            | "/moments/bundle"
            | "/moments/media"
            | "/moments/media/content"
            | "/moments/preserved"
            | "/moments/preserved/export"
            | "/moments/live"
            | "/moments/live/overview"
            | "/moments/live/export"
            | "/moments/live/cancel"
            | "/messages/live/search"
            | "/messages/live/search/cancel"
            | "/wechat/settings/read"
            | "/wechat/settings/plan"
            | "/messages/live/calendar"
            | "/messages/live/calendar/cancel"
            | "/messages/live/conversation"
            | "/messages/live/conversation/multi"
            | "/messages/live/conversation/cancel"
            | "/messages/live/conversation/media/content"
            | "/messages/recovery/media/content"
            | "/messages/live/export"
            | "/messages/recovery/export"
    );
    let read_post = read_post
        || input.body.as_ref().is_some_and(|body| match route {
            "/messages/preservation" => matches!(body.get("action").and_then(|v|v.as_str()), Some("status"|"list")),
            "/messages/recovery" => matches!(
                body.get("action").and_then(|v| v.as_str()),
                Some(
                    "list"
                        | "status"
                        | "page"
                        | "records"
                        | "timeline"
                        | "locate_message"
                        | "social_report"
                        | "media_catalog"
                        | "media_page"
                        | "media_progress"
                        | "transcript_progress"
                )
            ),
            "/articles" => matches!(body.pointer("/operation/action").and_then(|v|v.as_str()),Some("list"|"get"|"capture_links"|"history"|"images"|"image")),
            "/articles/jobs" => matches!(body.pointer("/operation/action").and_then(|v|v.as_str()),Some("plans"|"runs"|"get_run")),
            "/library/records" => body.get("action").and_then(|v| v.as_str()) == Some("list"),
            _ => false,
        });
    if read_only && input.method != "GET" && !(input.method == "POST" && read_post) {
        return Err(AppError::new("FORBIDDEN", "只读MCP工具不允许此操作").into());
    }
    let raw = input.body_hex.is_some();
    let bytes = if let Some(encoded) = input.body_hex {
        hex::decode(encoded)
            .map_err(|_| AppError::new("INVALID_ARGUMENT", "body_hex不是有效十六进制"))?
    } else if let Some(body) = input.body {
        serde_json::to_vec(&body).map_err(|_| AppError::internal())?
    } else {
        Vec::new()
    };
    let mut request = axum::http::Request::builder()
        .method(input.method.as_str())
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .header(
            header::CONTENT_TYPE,
            input.content_type.as_deref().unwrap_or(if raw {
                "application/octet-stream"
            } else {
                "application/json"
            }),
        );
    if let Some(key) = input.idempotency_key {
        request = request.header("idempotency-key", key);
    }
    let request = request
        .body(axum::body::Body::from(bytes))
        .map_err(|_| AppError::new("INVALID_ARGUMENT", "HTTP工具请求无效"))?;
    let response = super::api_routes(state.clone())
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
        .oneshot(request)
        .await
        .map_err(|_| AppError::internal())?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|_| AppError::new("QUOTA_EXCEEDED", "结果超过64MiB，请通过HTTP下载此媒体"))?;
    let body = serde_json::from_slice::<Value>(&bytes).ok();
    Ok(if let Some(body) = body {
        json!({"status":status.as_u16(),"content_type":content_type,"body":body})
    } else {
        json!({"status":status.as_u16(),"content_type":content_type,"body_hex":hex::encode(bytes)})
    })
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, AppError> {
    serde_json::from_value(value).map_err(|_| AppError::new("INVALID_ARGUMENT", "工具参数无效"))
}
fn value<T: serde::Serialize>(v: T) -> Result<Value, AppError> {
    serde_json::to_value(v).map_err(|_| AppError::internal())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    account_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    account_id: String,
    #[serde(default)]
    after: i64,
    #[serde(default = "default_limit")]
    limit: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendId {
    account_id: String,
    outbox_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendHistory {
    account_id: String,
    before: Option<i64>,
    #[serde(default = "default_limit")]
    limit: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    account_id: String,
    username: String,
    idempotency_key: String,
    text: Option<String>,
    asset_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipient {
    account_id: String,
    view_id: String,
    multi: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageInput<T> {
    account_id: String,
    message: T,
}
fn call_tool(app: &Application, p: &Principal, name: &str, args: Value) -> Result<Value, AppError> {
    match name {
        "wechat_settings_capabilities" | "wechat_settings_read" => {
            let q:wechat_settings::AccountInput=decode(args)?;
            if name=="wechat_settings_read" {Ok(json!({"data":app.wechat_settings_read(p,&q.account_id)?}))}else{Ok(json!({"data":app.wechat_settings_capabilities(p,&q.account_id)?}))}
        }
        "wechat_settings_plan" => {let q:wechat_settings::TargetInput=decode(args)?;Ok(json!({"data":app.wechat_settings_plan(p,&q.account_id,&q.target)?}))}
        "wechat_settings_apply" => {let q:wechat_settings::ApplyInput=decode(args)?;Ok(json!({"data":app.wechat_settings_apply(p,&q.account_id,&q.target,&q.idempotency_key)?}))}
        "wechat_settings_changes" => {let q:wechat_settings::ChangesInput=decode(args)?;Ok(json!({"data":wechat_settings::changes_value(app.wechat_settings_changes(p,&q.account_id,q.before,q.limit)?)}))}
        "wechat_settings_restore" => {
            #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct Restore {account_id:String,id:String,idempotency_key:String}
            let q:Restore=decode(args)?;Ok(json!({"data":app.wechat_settings_restore(p,&q.account_id,&q.id,&q.idempotency_key)?}))
        }
        "list_accounts" => {
            let _: Empty = decode(args)?;
            Ok(json!({"data":app.accounts(p)?}))
        }
        "get_capabilities" => {
            let _: Empty = decode(args)?;
            p.require("capabilities:read")?;
            Ok(json!({"data":app.capabilities()}))
        }
        "get_connection" => {
            let q: Account = decode(args)?;
            Ok(json!({"data":app.binding(p,&q.account_id)?}))
        }
        "get_messages" => {
            let q: Page = decode(args)?;
            if q.after < 0 {
                return Err(AppError::new("INVALID_ARGUMENT", "after 必须非负"));
            }
            let rows = app.received_texts(p, &q.account_id, q.after, q.limit)?;
            let next = rows.last().map_or(q.after, |row| row.position);
            Ok(
                json!({"data":rows,"next_cursor":next,"historical_complete":false,"receive_status":app.receive_cursor_local(&q.account_id)?,"health":app.receive_health_local(&q.account_id)?}),
            )
        }
        "get_events" => {
            let q: Page = decode(args)?;
            let rows = app.events(p, &q.account_id, q.after, q.limit)?;
            let next = rows.last().map_or(q.after, |row| row.event_id);
            Ok(json!({"data":rows,"next_cursor":next}))
        }
        "list_sends" => {
            let q: SendHistory = decode(args)?;
            value(app.send_history_page(p, &q.account_id, q.before, q.limit)?)
        }
        "get_send_status" | "submit_send" | "cancel_send" | "reconcile_send" => {
            let q: SendId = decode(args)?;
            value(match name {
                "submit_send" => app.submit_client_send(p, &q.account_id, &q.outbox_id)?,
                "cancel_send" => app.cancel_send(p, &q.account_id, &q.outbox_id)?,
                "reconcile_send" => app.reconcile_send(p, &q.account_id, &q.outbox_id)?,
                _ => app.send_status(p, &q.account_id, &q.outbox_id)?,
            })
        }
        "send_mention" => {
            let q: MessageInput<werc_application::mention_send::ClientMention> = decode(args)?;
            value(app.create_mention_send(p, &q.account_id, &q.message)?)
        }
        "send_quote" => {
            let q: MessageInput<werc_application::quote_send::ClientQuote> = decode(args)?;
            value(app.create_quote_send(p, &q.account_id, &q.message)?)
        }
        "prepare_recipient" => {
            let q: Recipient = decode(args)?;
            p.require("messages:execute")?;
            p.require_account(&q.account_id)?;
            value(app.prepare_recipient_local(
                &q.account_id,
                werc_application::recipients::RecipientSelection {
                    view_id: q.view_id,
                    multi: q.multi,
                },
            )?)
        }
        _ => {
            let q: Send = decode(args)?;
            p.require("messages:execute")?;
            if (name == "send_text" && (q.text.is_none() || q.asset_id.is_some()))
                || (name != "send_text" && (q.asset_id.is_none() || q.text.is_some()))
            {
                return Err(AppError::new(
                    "INVALID_ARGUMENT",
                    "文字请求仅接受text；媒体请求仅接受asset_id",
                ));
            }
            let asset = q.asset_id.as_deref().unwrap_or_default();
            value(match name {
                "send_text" => app.create_text_send(
                    p,
                    &q.account_id,
                    &q.username,
                    q.text.as_deref().unwrap_or_default(),
                    &q.idempotency_key,
                )?,
                "send_voice" => {
                    app.create_voice_send(p, &q.account_id, &q.username, asset, &q.idempotency_key)?
                }
                "send_image" => {
                    app.create_image_send(p, &q.account_id, &q.username, asset, &q.idempotency_key)?
                }
                "send_file" => {
                    app.create_file_send(p, &q.account_id, &q.username, asset, &q.idempotency_key)?
                }
                "send_video" => {
                    app.create_video_send(p, &q.account_id, &q.username, asset, &q.idempotency_key)?
                }
                "send_emoticon" => app.create_emoticon_send(
                    p,
                    &q.account_id,
                    &q.username,
                    asset,
                    &q.idempotency_key,
                )?,
                _ => return Err(AppError::new("INVALID_ARGUMENT", "未知工具")),
            })
        }
    }
}
