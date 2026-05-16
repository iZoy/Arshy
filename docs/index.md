# Arshy 文档

AI Agent 的原生 Shell 执行层。所有 shell 命令通过 MCP 协议执行，输出结构化。

## 快速开始

| 步骤 | 内容 | 时间 |
|------|------|------|
| [安装](getting-started/install.md) | cargo install 或从源码编译 | 2 分钟 |
| [第一条命令](getting-started/first-command.md) | arshy run echo hello | 1 分钟 |
| [接入 MCP](getting-started/mcp-setup.md) | 配置 Claude Code / Cursor | 2 分钟 |

## 指南

- [配置](guides/configuration.md) — 配置文件格式、环境变量、分层覆盖
- [自定义 Parser（TOML）](guides/custom-parser-toml.md) — 声明式正则匹配
- [自定义 Parser（Rhai）](guides/custom-parser-rhai.md) — 脚本化跨行解析
- [安全](guides/security.md) — 命令过滤、路径沙箱、权限分级、审计日志
- [Daemon 管理](guides/daemon-management.md) — 启停、PID、生命周期
- [故障排查](guides/troubleshooting.md) — 常见问题及解决

## 参考

- [CLI 命令](reference/cli.md) — 全部子命令及参数
- [配置 Schema](reference/config-schema.md) — 所有配置项、类型、默认值
- [MCP 协议](reference/mcp-protocol.md) — 工具定义、资源、通知
- [IPC 协议](reference/ipc-protocol.md) — JSON-RPC 方法、错误码、数据类型
- [Parser TOML 格式](reference/parser-toml-format.md) — TOML parser 定义规范
- [Parser Rhai API](reference/parser-rhai-api.md) — 脚本引擎 API 参考
- [错误码](reference/error-codes.md) — JSON-RPC 错误码及重试语义

## 深度理解

- [架构](explanation/architecture.md) — 组件、数据流、部署模型
- [设计原则](explanation/design-principles.md) — 六条核心约束
- [Parser 管线](explanation/parser-pipeline.md) — 检测→匹配→事件生成全链路

## 内置 Parser（20 个）

| 工具 | 工具 | 工具 | 工具 |
|------|------|------|------|
| tsc | cargo | jest | vite |
| eslint | go | python | cc (gcc/clang) |
| npm | webpack | prettier | swc |
| esbuild | clippy | make | gradle |
| cargo-test | mocha | pip | pnpm |
