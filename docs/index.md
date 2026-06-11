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
- [MCP 协议](reference/mcp-protocol.md) — 工具定义、资源、通知、retryable 错误
- [IPC 协议](reference/ipc-protocol.md) — JSON-RPC 方法、错误码、数据类型
- [Parser TOML 格式](reference/parser-toml-format.md) — TOML parser 定义规范（v1.0 schema）
- [Parser Rhai API](reference/parser-rhai-api.md) — 脚本引擎 API 参考
- [错误码](reference/error-codes.md) — JSON-RPC 错误码及重试语义

## 规范

- [Parser Specification v1.0](spec/parser-spec.md) — 开放规范：parser 定义格式、事件 schema、fixture 测试格式、贡献指南

## 深度理解

- [架构](explanation/architecture.md) — 组件、数据流、进程树关闭、并发模型
- [设计原则](explanation/design-principles.md) — 六条核心约束
- [Parser 管线](explanation/parser-pipeline.md) — 检测→格式检测→匹配→ReDoS 校验→事件生成全链路

## 核心能力

### Smart Sync Shell
Agent 跑命令，拿结构化结果。1 次 MCP 调用，无需 subscribe/query。

```
Agent: arshy_exec(command: "cargo test")
      ← {status:"completed", exit_code:0, summary:{...}, root_cause:null, events:[...]}
```

### 6 级 Parser 管道
```
行 → 格式检测(JSON/NDJSON/YAML/CSV) → Stateful(Rhai) → TOML(regex) → Crash(通用) → Heuristic(启发式) → Raw
```

### 智能输出
- **summary**: 按 type/severity 分组统计
- **root_cause**: 第一个 error 级别事件
- **project_context**: 失败时自动附加 git diff + 错误-变更关联
- **context**: 错误事件自动附带 ±3 行源码上下文
- **dedup**: 相同行自动折叠，减少 token 浪费
- **hint**: 错误事件自动附带修复建议（7 个精选错误码，4 种语言，仅保留 LLM 不易识别的）

## 内置 Parser（37 个）

| 工具 | Pattern | 工具 | Pattern | 工具 | Pattern |
|------|:------:|------|:------:|------|:------:|
| tsc | 4 | prettier | 4 | terraform | 5 |
| cargo | 3 | swc | 4 | kubectl | 5 |
| cargo-test | 7 | esbuild | 3 | helm | 5 |
| clippy | 3 | make | 4 | aws | 5 |
| eslint | 5 | gradle | 4 | docker | 5 |
| go | 4 | mocha | 4 | uv | 5 |
| python | 5 | pip | 5 | ruff | 4 |
| cc (gcc/clang) | 5 | pnpm | 4 | turbo | 4 |
| npm | 5 | vite | 4 | nx | 4 |
| webpack | 5 | jest | 3 | deno | 5 |
| biome | 4 | oxlint | 3 | bun | 5 |
| vitest | 4 | curl | 4 | git | 5 |
| ssh | 4 | | | | |

[↗ 查看全部 parser 规则](../../parsers/builtin/)

## 开发者工具

### 终端 UI
```bash
arshy run "cargo build" --format pretty    # 可视化输出（box drawing + color）
arshy run "cargo build" --format json      # 原始 JSON
arshy run "cargo build" --format auto      # 自动检测（TTY=pretty，管道=json）
```

### Benchmark
```bash
arshy benchmark                            # 跨 37 个 parser 的性能测试
scripts/benchmark.sh                       # 格式化输出脚本
```

### Parser 管理
```bash
arshy parser reload                        # 热重载 + 显示变更 diff
arshy parser list                          # 列出已加载的 parser
```
