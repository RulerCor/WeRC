//! Line-framed STDIO relay to the running local service. Never retries requests.
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
const MAX_LINE: u64 = 64 * 1024 * 1024;
fn run() -> Result<(), String> {
    let endpoint =
        std::env::var("WERC_MCP_URL").unwrap_or_else(|_| "http://127.0.0.1:5031/mcp".into());
    let url = reqwest::Url::parse(&endpoint).map_err(|_| "WERC_MCP_URL无效")?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "/mcp" | "/mcp/read")
    {
        return Err("WERC_MCP_URL必须是本机回环HTTP的/mcp或/mcp/read地址".into());
    }
    let token = std::env::var("WERC_API_TOKEN")
        .map_err(|_| "请在客户端环境变量WERC_API_TOKEN中设置授权令牌")?;
    if !(32..=256).contains(&token.len()) || token.chars().any(char::is_control) {
        return Err("WERC_API_TOKEN格式无效".into());
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(660))
        .build()
        .map_err(|_| "无法初始化本机HTTP客户端")?;
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "无法读取STDIO输入")?;
        if count == 0 {
            break;
        }
        if count as u64 > MAX_LINE {
            return Err("MCP请求超过64MiB".into());
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let request = serde_json::from_slice::<Value>(&line);
        let response = match request {
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
            Ok(request) => {
                let id = request.get("id").cloned();
                let result = (|| -> Result<Option<Value>, String> {
                    let response = client
                        .post(url.clone())
                        .bearer_auth(&token)
                        .header("Content-Type", "application/json")
                        .header("Accept", "application/json, text/event-stream")
                        .body(line)
                        .send()
                        .map_err(
                            |_| "本机MCP服务无法连接或请求超时；没有重试，请查询原幂等键的状态",
                        )?;
                    if id.is_none() && response.status() == reqwest::StatusCode::ACCEPTED {
                        return Ok(None);
                    }
                    let status = response.status();
                    let mut bytes = Vec::new();
                    response
                        .take(MAX_LINE * 2 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| "MCP响应读取失败；没有重试")?;
                    if bytes.len() as u64 > MAX_LINE * 2 {
                        return Err("MCP响应过大，请缩小查询或通过HTTP下载".into());
                    }
                    let value: Value =
                        serde_json::from_slice(&bytes).map_err(|_| "本机MCP服务未返回JSON响应")?;
                    if !status.is_success() {
                        return Err(format!("本机MCP服务拒绝请求（HTTP {}）", status.as_u16()));
                    }
                    Ok(Some(value))
                })();
                match result {
                    Ok(v) => v,
                    Err(message) => {
                        if let Some(id) = id {
                            Some(
                                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":message}}),
                            )
                        } else {
                            eprintln!("{message}");
                            None
                        }
                    }
                }
            }
        };
        if let Some(value) = response {
            serde_json::to_writer(&mut output, &value).map_err(|_| "STDIO输出失败")?;
            output
                .write_all(b"\n")
                .and_then(|_| output.flush())
                .map_err(|_| "STDIO输出失败")?;
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("WeRC MCP: {error}");
        std::process::exit(1);
    }
}
