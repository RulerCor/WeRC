# 第三方实现来源

## helper（仅非商业）与 MinHook（BSD）

`native/helper/`保存用户提供ZIP中未经修改的原生源码与许可，来源及逐文件摘要见`native/helper/SOURCE.json`。`native/message-bridge/common.inc`与`core_version.h`派生自该源码，公开接口、控制器和Rust装配为本工程实现。许可为`LicenseRef-helper-noncommercial-1.0`，允许按原许可进行非商业分发。本项目对来源于 helper 的本机桥接实现作了修改。源码许可原文是`native/helper/LICENSE`，分发运行目录另附`helper-NONCOMMERCIAL.txt`。不得将此派生成分视为MIT或可用于商业产品。

MinHook 1.3.4原文在`native/helper/native/vendor/minhook-1.3.4/LICENSE.txt`，运行目录附`MinHook-BSD.txt`。上游其他通知保留为`helper-THIRD-PARTY-NOTICES.md`。这里只打包从源码构建的适配组件，不打包微信DLL。

## SILK C核心与silk-rs来源

`native/silk/`固定官方crates.io `silk-rs 0.2.0`归档内的完整SILK C核心，来源摘要与文件清单见`native/silk/SOURCE.json`。核心版权为Skype Limited，适用`native/silk/Skype-LICENSE.txt`中的BSD式条款；原文明确不授予专利权。`wrapper-MIT.txt`仅记录发行物Rust封装的MIT许可，不能将C核心重新标为MIT。本项目未运行上游Rust构建脚本，而以自有`native/voice-codec/codec.cpp`及MSVC源码构建。运行分发附`Skype-SILK.txt`及`silk-rs-MIT.txt`原文。

## wcdb-key-tool（MIT）

`crates/wechat-win/src/key_acquisition.rs`的Config.Cipher标记、对象布局和候选XOR解码算法参考/改写自[TANGandXUE/wcdb-key-tool](https://github.com/TANGandXUE/wcdb-key-tool)，固定commit `79f1b5b92e12c66aa281b4a60a3c478b5f547dfa`的`wcdb_key_tool_windows.py`。许可全文见`wcdb-key-tool-MIT.txt`，Copyright 2025 CloudDreamAI / TANGandXUE。

本产品以Rust重写只读路径，另外增加指定安装文件与进程句柄核对、同用户与x64检查、有界扫描、逐来源HMAC认证、受保护保存和显式确认。没有移植调试器、权限提升、明文密钥输出或落盘逻辑。上游实现不构成本产品对特定微信版本兼容、账号归属或安全结果的保证。

依赖的其他许可元数据另见构建证据的dependency-inventory.json。此文件不是所有依赖的完整许可证审计结论。

## SQLCipher 与 OpenSSL

独立 werc-sqlcipher-worker 从 Cargo.lock 固定的 libsqlite3-sys 0.35.0 所带 SQLCipher 4.6.1，以及 openssl-src 300.6.1+3.6.3 所带 OpenSSL 源码编译。许可原文复制自对应发行源码，分别附于 SQLCipher-BSD.txt 和 OpenSSL-Apache-2.0.txt；配套运行目录及开发包同时携带。这是独立进程，与产品存储的普通SQLite分开链接。

## Zstandard文字解码

在线引擎固定使用ruzstd 0.8.3及其twox-hash 2.1.5依赖，均从Cargo.lock校验的Rust源码构建；只增加这两项依赖，未更新其余独立引擎版本。许可原文分别复制自对应crate发行物的LICENSE及LICENSE.txt，附于ruzstd-MIT.txt和twox-hash-MIT.txt。自有适配器限制输入、解压历史窗口、输出、单帧及校验和；未使用下载二进制。

## png 0.18.1

静态PNG完整解码与规范化写出，固定Cargo.lock。采用MIT许可，原MIT/Apache-2.0文本分别见`png-MIT.txt`、`png-APACHE.txt`。依赖清单及各自许可见dependency-inventory.json。

## quick-xml 0.42.0

有界接收媒体XML解析，固定版本与Cargo.lock；采用MIT许可，原文复制自该crate的LICENSE-MIT.md并保存为`quick-xml-MIT.txt`，随运行目录和开发包分发。解析不访问外部实体、URL或调用者路径。

## jpeg-decoder 0.3.2

接收JPEG完整解码，固定版本且禁用默认并行特性。采用MIT许可，保留MIT/Apache-2.0原文于`jpeg-decoder-MIT.txt`和`jpeg-decoder-APACHE.txt`，随运行目录及开发包分发。

## mp4parse 0.17.0

接收视频的 MP4 容器及样本表解析，固定版本、禁用默认特性，从 Cargo.lock 固定的 crate 源码构建。采用 MPL-2.0，增加 HEVC 容器条目接收适配；许可原文保存于 `mp4parse-MPL-2.0.txt`。本产品在调用前实施大小和表项限额，解析不下载媒体、不解码压缩帧。依赖来源与校验和保留在 Cargo.lock，分发时在 open-source-components/mp4parse-source.zip 提供该修改组件对应源码及许可，不包含主程序源码。

## GIF 0.13.3 / weezl 0.1.12 / color_quant 1.1.0

GIF 资产完整解码使用固定 gif 与 weezl 源码，MIT / Apache-2.0 双许可；gif 默认特性包含 color_quant（MIT）。未修改上游源码，许可原文随源码、便携包和安装器分发。依赖由 Cargo.lock 固定，无额外系统解码器或下载运行时。

## Standalone cached-animation decoder

`werc-emoticon-codec.exe` is a separate GPL-3.0-or-later program combining the GPL-3.0-or-later wrapper, libde265 1.1.3 (LGPL-3.0-or-later) and JBIG-KIT 2.2 (GPL-2.0-or-later). License texts are included here. The accompanying `open-source-components/werc-emoticon-codec-source.zip` supplies its complete corresponding source, CMake build files and protocol documentation; it can be rebuilt independently. No helper or WeRC application code is linked into this executable. The helper's separate noncommercial conditions continue to apply to the helper-derived components.

## Offline ASR

Whisper.cpp 1.9.5 and the Whisper tiny model use MIT licenses, included under licenses. The Windows x64 CPU sherpa-onnx 2.0.1 executable is the official static-MT release. Its Apache-2.0 license and ONNX Runtime, Kaldi, kaldi-native-fbank and KissFFT notices are included in asr/. CUDA components are optional user downloads and not embedded in this distribution. Paraformer test weights with unspecified redistribution terms are not distributed.
