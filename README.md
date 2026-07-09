# Swidex

Swidex 是一个 macOS 桌面小工具，用来在 Codex 的 ChatGPT 官方账号模式和 API / 中转站模式之间做可诊断、可回退的切换。

它解决的问题不是“管理所有 AI provider”，而是一个更窄但很容易卡住的场景：Codex 已经重新登录了官方账号，但 `~/.codex/config.toml`、shell 环境变量或 macOS `launchctl` 里仍残留旧的中转站配置，导致实际请求还可能继续走 API / 中转站。

## 核心功能

- **状态诊断**：检查 `codex login status`、`model_provider`、`base_url`、`env_key`、进程环境变量、shell 启动文件和 `launchctl`。
- **启用官方模式**：备份并清理中转站 provider、相关 API/Base URL 环境变量、shell 残留和 `launchctl` 残留。
- **启用中转站模式**：写入 `model_provider = "codex"` 和 `[model_providers.codex]`，并使用 `env_key = "CODEX_API_KEY"`。
- **中转站配置管理**：保存名称、备注、Base URL、API Key 和持久化偏好；保存配置不会自动启用，启用需要二次确认。
- **Key 安全**：API Key 加密保存到 macOS Keychain；localStorage 只保存中转站元数据和是否已有 Key 的标记，日志不会打印明文 key。
- **历史备份**：备份 `~/.codex/sessions/` 和 `~/.codex/history.jsonl` 到 `~/CodexBackups/`，不包含 `auth.json`。
- **同步对话历史**：可让官方账号与中转或定制供应商共用同一个历史 / 续聊列表，默认把现有官方会话一并同步进来；迁移/还原前会备份会话文件和 `state_5.sqlite`。
- **重置登录缓存**：二次确认后备份并删除 `~/.codex/auth.json`，用于需要重新登录的场景。
- **操作日志**：记录每次操作的动作、文件操作/命令、备份位置和执行结果摘要。

## 两种模式

### ChatGPT 官方账号模式

Swidex 会把 Codex 配置收敛到更干净的官方账号状态：

```toml
model_provider = "openai"
```

同时移除 `model_providers.codex` 里的第三方 `base_url` / `env_key`，并清理常见环境变量：

```text
OPENAI_API_KEY
CODEX_API_KEY
OPENAI_BASE_URL
OPENAI_API_BASE
BASE_URL
API_BASE_URL
CODEX_ACCESS_TOKEN
```

这个操作不会删除 `auth.json`，也不会主动让你退出登录。它和“重置登录缓存”不是同一件事。

如果开启了“同步对话历史”，官方账号会加入中转或定制供应商使用的共享历史列表：

```toml
model_provider = "codex"

[model_providers.codex]
name = "OpenAI"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = true
```

这只改变会话历史归类标签，不改变当前请求通道。认证仍走 ChatGPT 登录，不写入中转站 `base_url` 或 API Key；从官方切到中转、或从中转切回官方时，看到的都是同一个历史 / 续聊列表。

### API / 中转站模式

Swidex 会写入或更新：

```toml
model_provider = "codex"

[model_providers.codex]
name = "codex"
wire_api = "responses"
requires_openai_auth = false
base_url = "https://example.com/v1"
supports_websockets = false
env_key = "CODEX_API_KEY"
```

填入软件里的 API Key 会加密保存到 macOS Keychain。只有在你明确勾选持久化时，Swidex 才会把 `OPENAI_BASE_URL`、`OPENAI_API_KEY` 和 `CODEX_API_KEY` 额外写入 `~/.zshrc`，并同步到 macOS `launchctl` 供后续 GUI 进程继承。

## 和 CC Switch 的关系

Swidex 的灵感来自 CC Switch 这类 provider 管理工具，但目标不同。

一句话区别：

```text
CC Switch 是通用供应商管理器 + 本地代理路由器；
Swidex 是针对 Codex 的状态清理器 + 登录模式切换器。
```

CC Switch 更适合管理多个工具、多 provider、本地代理、模型目录、MCP/Skills 等复杂场景。Swidex 则刻意做窄：直接围绕 `~/.codex/config.toml`、`auth.json`、shell 启动文件、环境变量、`launchctl` 和 `codex login status` 做诊断与清理。

这也意味着 Swidex 不追求替代 CC Switch。它更像一个 Codex Doctor：当你只想确认“Codex 现在到底会走官方账号还是中转站”时，Swidex 给出一个更直接、可审计的答案。

## 安全原则

- 修改文件前先备份。
- 不默认删除 `auth.json`。
- 不删除 `sessions/` 和 `history.jsonl`。
- 不在日志里打印完整 API Key。
- 保存中转站信息不会自动启用。
- 备份历史不会打包登录凭据。
- 同步对话历史只改 `model_provider` 归类标签，不切换当前请求通道；关闭时只还原能按备份分清来源的会话，开启期间新建且分不清来源的会话会留在中转或定制供应商历史里。
- 跨供应商续聊旧会话仍可能因为 `encrypted_content` 无法被另一后端解密而失败。

## 开发

```bash
npm install
npm run tauri:dev
```

## 打包

```bash
npm run tauri build
```

常见产物：

```text
src-tauri/target/release/bundle/macos/Swidex.app
src-tauri/target/release/bundle/dmg/Swidex_0.1.2_aarch64.dmg
```

### 本地分享给朋友

没有 Apple Developer ID 时，Tauri 产物通常只能做 ad-hoc 签名，`spctl` 仍会因为没有开发者证书和公证而拒绝。这不是 app group entitlement 问题；Swidex 没有 SwiftData/app group 依赖。

如果只是临时发给可信朋友测试，可以先把 DMG 里的 app 清理扩展属性并完整 ad-hoc 重签：

```bash
bash scripts/resign-local-dmg.sh
```

脚本会生成：

```text
src-tauri/target/release/bundle/dmg/Swidex_0.1.2_aarch64-local-ad-hoc.dmg
```

朋友安装到 `/Applications` 后，如果 macOS 仍提示无法打开，可以在朋友机器上执行：

```bash
xattr -dr com.apple.quarantine /Applications/Swidex.app
open /Applications/Swidex.app
```

这只是 Gatekeeper 绕过方式，适合小范围可信测试。正式分发仍应使用 Developer ID 签名并公证。

## 验证

```bash
npm run build
npm run test:rust
npm run tauri build
```
