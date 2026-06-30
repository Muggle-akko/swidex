# Swidex

Swidex 是一个 macOS 桌面小工具，用来在 Codex 的 ChatGPT 官方账号模式和 API / 中转站模式之间做可诊断、可回退的切换。

它解决的问题不是“管理所有 AI provider”，而是一个更窄但很容易卡住的场景：Codex 已经重新登录了官方账号，但 `~/.codex/config.toml`、shell 环境变量或 macOS `launchctl` 里仍残留旧的中转站配置，导致实际请求还可能继续走 API / 中转站。

## 核心功能

- **状态诊断**：检查 `codex login status`、`model_provider`、`base_url`、`env_key`、进程环境变量、shell 启动文件和 `launchctl`。
- **启用官方模式**：备份并清理中转站 provider、相关 API/Base URL 环境变量、shell 残留和 `launchctl` 残留。
- **启用中转站模式**：写入 `model_provider = "codex"` 和 `[model_providers.codex]`，并使用 `env_key = "CODEX_API_KEY"`。
- **中转站配置管理**：保存名称、备注和 Base URL；保存配置不会自动启用，启用需要二次确认。
- **Key 安全**：API Key 默认只用于本次启用，不写入 shell 文件；界面支持显示/隐藏和复制，但日志不会打印明文 key。
- **历史备份**：备份 `~/.codex/sessions/` 和 `~/.codex/history.jsonl` 到 `~/CodexBackups/`，不包含 `auth.json`。
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

API Key 默认不会长期写入 `~/.zshrc`。只有在你明确勾选持久化时，Swidex 才会写入 `CODEX_API_KEY`。

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
src-tauri/target/release/bundle/dmg/Swidex_0.1.0_aarch64.dmg
```

## 验证

```bash
npm run build
npm run test:rust
npm run tauri build
```
