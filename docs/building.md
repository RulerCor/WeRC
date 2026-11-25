# 从源码构建 / Building from source

完整桌面版需要 Windows x64、Visual Studio C++ Build Tools（含 Windows SDK）、Node.js 24/npm、Python 3、CMake、Ninja 和原生 Windows Perl（SQLCipher/OpenSSL）。使用 `rustup` 安装 Rust；项目的 `rust-toolchain.toml` 将选定工具链。微信和 WebView2 运行时按需安装。安装器构建需要联网获取上游依赖和语音资源。

The full desktop build requires Windows x64, Visual Studio C++ Build Tools with the Windows SDK, Node.js 24/npm, Python 3, CMake, Ninja, and native Windows Perl for SQLCipher/OpenSSL. Install Rust through `rustup`; the repository pins the toolchain. The installer build downloads upstream dependencies and speech resources. WeChat is installed independently.

```powershell
npm ci --ignore-scripts
npm run desktop:build
```

安装器生成在 `target/release/bundle/nsis/`。首次构建会编译原生组件、生成离线 API 文档，并按摘要下载固定版本的语音模型和 Sherpa 运行时。 / The installer is written to `target/release/bundle/nsis/`. The build compiles native components, generates offline API docs, and fetches pinned speech assets verified by hash.

前端预览：`npm run dev`，无需微信或原生组件。桌面开发：先运行 `./scripts/build-from-source.ps1 -PrepareOnly`，再运行 `npm run desktop:dev`。 / For a browser-only preview, run `npm run dev`. For desktop development, run `./scripts/build-from-source.ps1 -PrepareOnly`, then `npm run desktop:dev`.

- `apps/desktop/`：界面与 Tauri 应用 / UI and Tauri app
- `crates/`：API、MCP、消息处理和资料读取 / API, MCP, messaging and local data
- `native/`、`workers/`：原生适配、语音及数据组件 / Native bridge, codecs and data workers
- `contracts/`：[OpenAPI 契约 / OpenAPI contract](../contracts/openapi.json)

官方发布 / Official release: run `scripts/prepare-update.py` with the finished distribution directory and the official signing key (`TAURI_SIGNING_PRIVATE_KEY_PATH`, with its `.pub` file), or let `scripts/package-v1.py publish` invoke it. Upload `latest.json` with both signed distributions to the same GitHub Release before making it public.
