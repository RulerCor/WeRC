# Bot 客户端接入

平台传递请求、原生消息和结果。会话白黑名单、免打扰、自动回复、模型、定时业务由 Bot 设置；平台不配置第二套策略。桌面资料浏览与导出不受影响。

## 一次连接

1. 首次进入跟随全屏设置，登录电脑微信后点击“已登录，自动连接”，自动登记、绑定、核验身份并准备资料。
2. 在引导中点击“启动并授权 Bot 收发”，自动开启本机服务并生成连接配置；也可稍后在主导航“Bot 接入”完成。
3. 复制配置到客户端并确认已保存。需要单独的 MCP 或 STDIO 格式时，在“Bot 接入”生成并复制对应配置。账号、地址、消息/结果流和令牌一起提供；无需逐项勾选六种消息权限。

只有本次明确签发的 Bot 令牌含 `messages:execute`。既有只读/草稿令牌不升级；自选权限仍可只创建草稿。Bot 预设不授权朋友圈发布、账号管理或磁盘任意路径操作。客户端配置含令牌，只保存到自己的客户端。

`GET /api/v1/connections` 返回本地当前资料槽位；其中 `identity_verified` 在契约中固定为 `false`，不是实时微信登录验证结果。不要据此判定原生通道一定未连接；具体操作仍由当前原生连接、能力与接口结果核验。

## 发送与结果

`POST /api/v1/messages/send-text`，Authorization 使用 Bearer 令牌，Idempotency-Key 为16–128个可见ASCII字符。JSON为 `{"account_id":"配置中的账号ID","username":"精确微信账号或群ID","text":"消息内容","wait_seconds":0}`。username是精确标识，不根据昵称猜目标。Bot授权路径自动准备目标、持久保存并批准排队，桌面无需再点击。

语音/图片/文件/视频/表情先使用对应 `/media/*` 上传，再调用 `messages/send-voice`、`send-image`、`send-file`、`send-video`、`send-emoticon`，提交 account_id、username、asset_id 和独立幂等键。格式限制以能力返回和具体接口为准。原生不支持时报告错误，不改为其他类型。

重试同一请求必须复用幂等键；同键不同内容或目标返回冲突。已存在任务在断连后也返回原状态，不因重新连接再发一遍。`GET /api/v1/outbox/{id}?account_id=...`读取状态；`POST /api/v1/outbox/{id}/submit`仅能提交本令牌自己的草稿。已经提交、结果未知或已完成时只返回原状态。收到200或原生受理不表示手机已送达。

客户端可监听 `/api/v1/events/stream?account_id=...` 的 outbox.updated，再读取任务状态。排队由现有发送执行器处理；平台关闭时不会发送。撤销/过期的客户端令牌会阻止尚未提交的任务。

`POST /api/v1/outbox/{id}/reconcile` 的JSON为 `{"account_id":"账号ID"}`，允许有 operations:read 的客户端主动回查。后台优先处理待发送的任务；发送空闲时逐条回查 Unknown/ServerAccepted，继续遍历历史。只有状态或证据变化才通知 outbox.updated。回查既不重新发送，也不续期批准。连接身份改变、原生回执丢失或提供方没有送达凭据时，结果仍可能未知。

### 群成员提醒与引用

`POST /api/v1/messages/send/mention`：

```json
{"account_id":"账号ID","message":{"username":"精确群账号@chatroom","members":["成员账号"],"text":"消息内容","idempotency_key":"唯一请求键至少16字符"}}
```

members为1至8个不同的精确账号，不接受昵称或@所有人。客户端决定提醒谁，平台检查格式并按指定ID构造原生提醒，不把字符串格式检查声称为实时群成员校验。桌面成员选择流程仍会核对已读取的成员页。

`POST /api/v1/messages/send/quote`：

```json
{"account_id":"账号ID","message":{"username":"精确会话账号","server_id":"原消息完整服务器ID","selection":{"view_id":"当前会话视图ID","local_id":"原消息本地ID","source":null},"text":"回复内容","idempotency_key":"另一唯一请求键至少16字符"}}
```

`selection` 调用保留。首次请求先读取会话视图；多分片视图必须提供该行的 `source`。服务端核对会话和完整服务器 ID，相同幂等键的重试可在原视图过期后返回已有任务。

若刚通过原生消息流收到来信，可以直接使用该事件的 SSE `id`（转为整数 `position`），无需读取会话视图：

```json
{"account_id":"acct_example","message":{"username":"example_group@chatroom","position":4821,"server_id":"9007199254741999","text":"收到，谢谢。","idempotency_key":"quote-example-request-01"}}
```

`account_id` 使用订阅时的账号，`username` 使用事件的 `payload.route.conversation`；私聊填对端 ID，群聊填群 ID。可省略 `server_id`；如提供，必须与 `payload.message.native_message_id` 完全相同，请保留字符串精度。`position` 和 `selection` 只能选择一个。

位置方式只支持留有已认证路由和有效服务器 ID 的私聊、群聊普通文字来信。服务端按位置精确读取，再按会话与完整服务器 ID 重新定位原文并校验原文哈希。群聊的微信传输前缀会在校验正文前去除，无需客户端处理。位置、账号、目标不符或消息类型不支持时拒绝，来源无效返回 `QUOTE_SOURCE_INVALID`；没有原生引用能力时返回 `NATIVE_FEATURE_UNAVAILABLE`。进入后台后若原文查不到、无法唯一确认或身份变化，任务会失败，不会按正文搜索或改发普通文字。可通过 `/api/v1/capabilities` 检查 `messages.send_quote`。

两种引用方式均需要 `messages:send`、`messages:read`、`operations:create`，`messages:execute` 决定是否自动排队。响应表示请求已进入发送流程；是否送达请以接收端为准。重试使用同一幂等键。

## 接收和续传

`GET /api/v1/messages/native/stream?account_id=...&after=0` 返回当前账号全部已持久原生消息，不按会话或免打扰过滤。事件 `message.received` 的 id 是产品消息 position。Bot完成处理后保存这个id，重连时使用 Last-Event-ID；默认从已有记录最早位置补发。客户端负责自己的去重和业务处理。

`receive.status` 报告连接epoch、累计缺口和丢弃数，不表示断连历史已经全部补齐；不支持或未观测到的微信消息不能凭空转发。`receive.status.health` 给出原生连接和接收保存错误；旧归档或未装配状态提供方时为null。内部存储/密钥保护暂时失败时保留原生待确认消息并重试，身份或协议错误仍会断开。历史累计不再使用10万条/256MiB停机门槛，每条大小和格式校验仍保留，磁盘写入失败不会确认消费。`stream.closed` 报告权限/读取错误。凭据失效立即结束，不能把故障当成功空结果。

短轮询可使用 `/api/v1/messages/native?account_id=...&after=0`，用返回的 next_cursor 继续。平台事件流的 event_id 与消息流的 position **是两个不同游标**，分别保存。不能用原生 sequence 代替跨重连位置。

## 历史补读与导出

- 看自动补读进度：`GET /api/v1/messages/recovered/status?account_id=账号ID`。按位置取已保存记录用 `GET /api/v1/messages/recovered`，持续订阅用 `/api/v1/messages/recovered/stream`。补读仅覆盖本机微信已保存的数据，重叠结果按 `id` 和 `content_digest` 合并。
- 建立可导出归档：向 `POST /api/v1/messages/recovery` 发送 `{"action":"start","account_id":"账号ID"}`，可加 Unix 秒级 `from`、`until`。省略 `view_id` 时使用当前登录已完成首轮后台扫描的本地分片；若返回 `RECOVERY_SOURCES_PENDING`，看自动补读状态再试。保留返回的 `id`，用 `action: status` 查看进度，用 `action: step` 携带 `id` 和唯一 `request_id` 逐页推进，直到 `complete_selected_sources` 为真。暂停时停止发 `step`，续跑使用原 `id`。若建任务后响应丢失，先用 `list` 查找之前建立的任务。
- 导出：`POST /api/v1/messages/recovery/export`，传 `account_id`、归档 `id`、`format` 和 `request_id`，按需传 `options`；HTTP 返回文件。已有归档可以在微信断开时导出。MCP 用 `api_read` 查状态、列表、读取和导出，用 `api_call` 建立和推进归档；持续流直接走 HTTP。建归档、推进及导出额外要求 `exports:create`，其余权限按 OpenAPI 中对应操作说明。

## MCP

Streamable HTTP入口 `/mcp`，只读入口 `/mcp/read`。支持协议2025-06-18；初始化、ping、tools/list、tools/call；无会话状态，不需要额外session配置。POST返回JSON，通知返回202；GET不提供MCP SSE（原生消息使用上面的专用流）。能力范围按令牌过滤，只读入口拒绝写工具。

工具包含账号、连接、能力、消息、事件、发送列表/状态、取消/提交/回查、六类普通发送和 send_mention/send_quote。媒体先通过HTTP上传，工具引用asset_id；引用的首次会话读取使用已有HTTP接口。HTTP配置可由支持Streamable HTTP的客户端直接使用；随包提供独立 `werc-mcp-stdio` 可执行转接器。客户端通过环境变量 `WERC_MCP_URL` 和 `WERC_API_TOKEN` 指定当前回环地址和令牌；命令需位于客户端的可执行搜索路径中，或在客户端选择程序位置。转接器只用STDIO输出JSON消息，不在响应丢失时自动重试；无需Node/Python运行时。

## 支持范围与结果含义

本文件描述 v1.1.0 接口行为，具体格式和兼容范围见 release-notes.md。普通文字、图片、语音、附件、视频、表情、引用和成员提醒已接精确原生编号、加密回执与跨连接只读回查。缺少可证明的来源或接收端凭据时仍保留未知；来源补读不把无法证明的断连缺口标为完整。兼容与使用限制见版本说明。

发送历史通过 `/api/v1/outbox/history` 或 MCP `list_sends` 完整分页，next_cursor作为before。512限额仅针对待批准/排队/提交中的请求；已提交的未知和历史结果保留且不占该额度。Bot 令牌默认有效 90 天，可在签发时选择短期、自定义时长（最短 1 秒）或永久。所有令牌均可随时撤销；没有逐会话配置。

## 并发读取与 BUSY

联系人、会话、搜索等独立在线查询可以并行。在线查询按本机逻辑 CPU 数分配 2–8 个名额，与较重的离线归档额度分开；额度包含等待中的请求。取钥、切换来源和替换会话窗口等共享状态操作保持互斥，冲突时按顺序等待最长约 30 秒；等待时可通过对应取消接口取消，后台补读会让位。达到额度或等待超时返回 BUSY（HTTP 429）。客户端宜限制同时读取数量；只读请求可延迟后重试，避免持续并发重试。连接、取钥、发送、上传和导出等各自有独立限制，不能对全部 BUSY 统一重放请求。发送重试保留原幂等键，先查原任务状态。


## 微信后台原生设置（v1.2.0）

v1.2.0 提供本节接口，仅通过微信后台原生入口提供设置，不激活窗口、不模拟输入、不采用 UIA。已验证 x64 微信 4.1.15.13 原版及 4.2.0.52 的 `new_message_sound`（新消息提示音）、`call_sound`（通话提示音）和 `auto_download`（自动下载总开关）。自动下载保留微信自己的大小、网络、媒体和缓存策略。按实际核心 DLL 的版本、架构与 SHA256 自动选择逐项验证过的能力，其他构建和项目不开放。

新令牌需要 `wechat:settings:read`；写入需在 Bot 接入中主动勾选“允许调整微信设置”，获得 `wechat:settings:write`。默认不授权写入，旧令牌不自动增权。设置不改变 Bot 消息流或回复策略。

先调用 `wechat_settings_capabilities`，再调用 `wechat_settings_read` 或 `wechat_settings_plan`，最后按授权直接 `wechat_settings_apply`。HTTP 对应 `/api/v1/wechat/settings/capabilities`（GET）、`read`、`plan`、`apply`（POST）；另有 `changes`（GET）和 `changes/{id}/restore`（POST）。MCP 对应 `wechat_settings_changes`、`wechat_settings_restore`。只读入口拒绝 apply/restore，通用工具遵守相同规则。

apply 参数示例：

```json
{"account_id":"当前账号ID","target":{"key":"new_message_sound","value":false,"expected_value":true},"idempotency_key":"settings-sound-off-0001"}
```

每次一个明确布尔目标；`expected_value` 可省略。结果在 `data.result`，`verified` 为微信原生复读匹配、`unchanged` 为已是目标且未写、`conflict` 为期望旧值不匹配且未写、`unknown` 为结果未确认。成功只证明设置值，不表示已经播放声音或下载媒体。幂等键为 16–128 个可见 ASCII 字符，按令牌和账号持久保存；同键同目标返回原记录且 `replayed=true`，不重复提交，同键不同目标返回 409。unknown 或程序中断留下的 SETTINGS_RESULT_PENDING 先 read，不换新键盲目重试。实时状态重新 read，不把原记录当成当前值。

restore 请求带 `account_id`、新的 `idempotency_key`；MCP 另带记录 `id`。仅已验证且原值已知的记录能恢复，必须仍是原连接、原构建，且当前值与原 observed 一致，否则返回冲突。字段、分页和响应结构见离线 API 文档/OpenAPI。
