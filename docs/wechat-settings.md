# 微信后台原生设置

此功能随 v1.2.0 正式发布，通过 HTTP 或 MCP 调用。

本版已验证 Windows x64 微信 **4.1.15.13 原版和 4.2.0.52** 的下列三项设置。连接时自动核验实际加载的核心版本、架构和摘要，再逐项开放能力。已有补丁版的收发支持保留，其设置尚未开放；其他未验证构建不会套用旧设置地址。

| key | 设置 | 范围 |
| --- | --- | --- |
| `new_message_sound` | 新消息提示音 | 当前登录的本机微信原生偏好 |
| `call_sound` | 通话提示音 | 当前登录的本机微信原生偏好 |
| `auto_download` | 自动下载总开关 | 保留微信自己的媒体类型、大小、网络及缓存策略；不主动下载历史文件 |

仅使用后台原生读写，不激活窗口、不模拟输入，也不使用 UIA 或前台回退。未找到可靠后台原生入口的设置不开放。以上设置不会更改 Bot 消息流、回复策略或 Windows 系统权限。`verified` 表示微信原生设置复读与目标一致，不表示已实际播放声音、收到通话或发生媒体下载。

## 授权与调用

先在桌面连接当前微信。Bot 接入页签发新令牌时，可勾选“允许调整微信设置”；默认不勾选。新令牌的读取权限为 `wechat:settings:read`，写入权限为 `wechat:settings:write`。已有令牌不自动增加权限。

| HTTP | MCP 工具 | 用途 |
| --- | --- | --- |
| `GET /api/v1/wechat/settings/capabilities?account_id=…` | `wechat_settings_capabilities` | 当前精确构建实际开放的设置；未连接或未适配时 `available=false`、`items=[]` |
| `POST /api/v1/wechat/settings/read` | `wechat_settings_read` | 读取当前设置值、实际核心指纹和连接标识 |
| `POST /api/v1/wechat/settings/plan` | `wechat_settings_plan` | 预览一个目标的差异，不修改设置 |
| `POST /api/v1/wechat/settings/apply` | `wechat_settings_apply` | 设置一个明确目标并原生复读 |
| `GET /api/v1/wechat/settings/changes?account_id=…` | `wechat_settings_changes` | 分页读取本账号变更记录；`limit` 为 1–100，默认 20，下一页传 `before=next_cursor` |
| `POST /api/v1/wechat/settings/changes/{id}/restore` | `wechat_settings_restore` | 恢复一条已验证记录的原值 |

所有 HTTP 成功结果放在 `data` 中。HTTP/MCP 共用鉴权和执行逻辑；`/mcp/read` 以及 `api_read` 拒绝设置写入，允许读取和预览。

读取请求：

```json
{"account_id":"当前账号ID"}
```

关闭新消息提示音的 apply 请求：

```json
{
  "account_id":"当前账号ID",
  "target":{"key":"new_message_sound","value":false,"expected_value":true},
  "idempotency_key":"settings-sound-off-0001"
}
```

每次一个布尔目标；多个设置依次调用。`expected_value` 可省略；提供时原生操作会重新读取，当前值不匹配就返回 `conflict`，不覆盖。plan 请求结构相同但不带 `idempotency_key`。

| `result.status` | 含义 |
| --- | --- |
| `verified` | 执行差异后，微信原生读取值等于目标 |
| `unchanged` | 原生当前值已经等于目标，没有写入 |
| `conflict` | `expected_value` 不匹配，没有写入 |
| `unknown` | 未能确认结果；先 read 确认当前值，不盲目换新幂等键重试 |

变更返回 `id`、精确构建、`native_epoch`、`result.before/requested/observed` 等。幂等键为 16–128 个可见 ASCII 字符，按账号和令牌持久保存；同键同目标返回原记录并标记 `replayed=true`，不会再次调用微信写入，同键不同目标返回 HTTP 409。原记录描述上次结果，实时值应重新 read。程序中断时保留 `unknown / SETTINGS_RESULT_PENDING`，再次请求不会重新提交。

HTTP restore 请求：

```json
{"account_id":"当前账号ID","idempotency_key":"settings-sound-restore-01"}
```

MCP restore 参数额外包含 `id`。恢复同样生成一条新记录，只允许旧值已知且结果为 `verified/unchanged` 的记录；当前连接、精确构建必须仍一致，当前设置值必须等于原来的 `observed`。微信连接变化或人工改过值时，先 read，再明确指定新的 apply 目标。

完整字段和错误响应见 OpenAPI 及随包离线 API 文档。无需逐条回到桌面确认已授权的普通设置。
