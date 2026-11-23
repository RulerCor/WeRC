# WeRC v1.2.5

适用于 Windows 10 1903 及以上 / Windows 11 x64。首次运行按全屏引导连接微信，可选授权 Bot；应用密码默认关闭。支持消息收发、资料浏览与导出、离线归档、语音转写及 HTTP / MCP 接入。

## 本版更新

- 微信核心尚未加载时，自动连接会等待并重试。连接通过后恢复之前启用的本机 API。
- 诊断记录连接结果，区分组件缺失、无法读取和校验失败；微信未连接时暂停后台补读。
- 导出没有建议文件名时提供默认名，补齐三个通讯录接口的 OpenAPI 操作标识。

- WeRC retries if WeChat is still loading its core module and restores the local API after reconnecting.
- Diagnostics record connection results and distinguish missing, unreadable, and mismatched components. Background history recovery pauses while disconnected.
- Export dialogs supply a default filename when needed. Three contact endpoints now have OpenAPI operation IDs.

## 兼容与使用限制

- 原生收发支持微信 4.2.0.53（推荐）、4.2.0.52、4.1.15.13 和 4.1.13.7 的匹配 x64 构建。4.1.15.9 可识别适配档案，未开放原生收发。微信设置接口仅对匹配的 4.2.0.53、4.2.0.52 和原版 4.1.15.13 开放。各项能力以连接页检测为准；微信需自行安装并登录。
- 在线资料来自本机缓存，多分片逐个读取，可能不含完整历史。媒体缺失会单独提示。交易、红包、好友申请中的未知状态保留来源值，不能据此判断到账或关系变化。
- 独立在线查询支持并行，与取钥、切换来源等操作冲突时短时排队，最多等待约 30 秒，支持取消。达到并发上限或等待超时仍可能返回 BUSY；稍后重试只读请求即可。发送请求必须复用原幂等键，结果未知时先查原任务，避免重复发送。
- “服务端接受”与“接收端确认”是不同状态；未取得送达凭据时会保留未知。
- 默认附带 Whisper tiny 与 CPU 转写组件。其他模型可自行导入；CUDA 按需启用和下载，效果取决于设备与模型，不保证快于 CPU。Windows Hello 需要设备支持。
- 采用手动更新，暂不提供自动更新。退出程序后替换完整程序目录，保留自己的资料目录；建议先做加密备份。
- 仅限非商业用途。分享时保留 `licenses` 和 `open-source-components` 中的许可证及对应开源组件资料。

首次使用见 [使用说明](usage.md)；API 接入见 程序目录内的 `API接口文档.html` 和 [Bot 接入说明](bot-client.md)。
