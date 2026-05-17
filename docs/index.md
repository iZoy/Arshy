# Arshy 文档

AI Agent 的原生 Shell。安装即接管 — 无需修改 CLAUDE.md，无需手动配置。

## 快速开始

| 步骤 | 内容 | 时间 |
|------|------|------|
| [安装](getting-started/install.md) | Plugin 安装 + daemon 部署 | 2 分钟 |
| [第一条命令](getting-started/first-command.md) | 通过 arshy_exec 执行命令 | 1 分钟 |
| [接入 MCP](getting-started/mcp-setup.md) | 自动注册，自描述初始化 | 0 配置 |

## 指南

- [配置](guides/configuration.md) — 配置文件格式、环境变量、分层覆盖、版本化、安全边界
- [自定义 Parser（TOML）](guides/custom-parser-toml.md) — 声明式正则匹配、deprecated/replaced_by 生命周期
- [自定义 Parser（Rhai）](guides/custom-parser-rhai.md) — 脚本化跨行解析
- [安全](guides/security.md) — 命令过滤、沙箱路径、权限分级、审计日志、socket 权限
- [Daemon 管理](guides/daemon-management.md) — 启停、PID、空闲退出、健康检查退避
- [故障排查](guides/troubleshooting.md) — 常见问题及解决

## 参考

- [CLI 命令](reference/cli.md) — 全部子命令及参数
- [配置 Schema](reference/config-schema.md) — 所有配置项、类型、默认值
- [MCP 协议](reference/mcp-protocol.md) — 工具定义、资源、通知、experimental 能力、retryable 错误
- [IPC 协议](reference/ipc-protocol.md) — JSON-RPC 方法、错误码、数据类型
- [Parser TOML 格式](reference/parser-toml-format.md) — TOML parser 定义规范（v1.0 schema）
- [Parser Rhai API](reference/parser-rhai-api.md) — 脚本引擎 API 参考
- [错误码](reference/error-codes.md) — JSON-RPC 错误码及重试语义

## 深度理解

- [架构](explanation/architecture.md) — 组件、数据流、进程树关闭、并发模型
- [设计原则](explanation/design-principles.md) — 六条核心约束
- [Parser 管线](explanation/parser-pipeline.md) — 检测→匹配→ReDoS 校验→事件生成全链路

## 内置 Parser（20 个，31 个 pattern）

| 工具 | Pattern | 工具 | Pattern |
|------|:------:|------|:------:|
| tsc | 4 | prettier | 4 |
| cargo | 3 | swc | 4 |
| cargo-test | 7 | esbuild | 3 |
| clippy | 3 | make | 4 |
| eslint | 5 | gradle | 4 |
| go | 4 | mocha | 4 |
| python | 5 | pip | 5 |
| cc (gcc/clang) | 5 | pnpm | 4 |
| npm | 5 | vite | 4 |
| webpack | 5 | jest | 3 |

[↗ 查看全部 parser 规则](../../parsers/builtin/)
