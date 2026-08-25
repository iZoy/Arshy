# arshy 文档

**arshy** 是 AI Agent 的结构化执行层：Agent 跑命令拿结构化结果（`file:line` 定位、
错误码、源码上下文），短命令零开销直出、长命令结构化解析、执行历史可查询、
按需自启 + 空闲退出、卸载零残余。

文档按 Diátaxis 框架组织为四个象限，另有产品与设计文档、发布清单与指标证据。

## 快速开始（Tutorials · 学习导向）

按顺序走完即可获得完整可用环境：

1. [安装 arshy 并验证](tutorials/install.md) — 一行安装 / 源码构建 / macOS codesign / `doctor` 体检
2. [运行第一条命令](tutorials/first-command.md) — 短命令直出、长命令结构化、任务管理
3. [把 Codex（或其他 Agent）接入 arshy](tutorials/setup-agent.md) — `setup` / `init` / 验证 / 零残余卸载

## 操作指南（How-to · 任务导向）

| 任务 | 指南 |
|------|------|
| 配置 arshy | [configure.md](how-to/configure.md) |
| 管理 daemon（启停/自启/空闲退出/launchd） | [manage-daemon.md](how-to/manage-daemon.md) |
| 为任意 CLI 编写自定义 parser | [create-parser.md](how-to/create-parser.md) |
| 接入各种 AI Agent | [integrate-agent.md](how-to/integrate-agent.md) |
| 配置与使用安全能力 | [security.md](how-to/security.md) |
| 运行与扩展测试体系 | [run-tests.md](how-to/run-tests.md) |
| 常见问题排查 | [troubleshoot.md](how-to/troubleshoot.md) |

## 参考资料（Reference · 信息导向）

- [CLI 命令](reference/cli.md) — 全部子命令、参数、默认值、退出码
- [配置参考](reference/config.md) — 配置键、环境变量、默认值、加载优先级
- [MCP 协议](reference/mcp.md) — 工具 schema、资源、通知、版本协商
- [IPC 协议](reference/ipc.md) — JSON-RPC 方法、数据结构、错误码
- [Parser 参考](reference/parsers.md) — 37 个内置 parser、TOML schema、fixture 约定
- [错误码参考表](reference/reference-codes.md) — 非显而易见退出码的含义（docker/kubectl/aws）
- [指标定义](reference/metrics.md) — `daemon/stats` / `daemon/analyze` / `benchmark` 字段含义、诚实边界与引用守则

## 概念解释（Explanation · 理解导向）

- [系统架构](explanation/architecture.md) — 双二进制、通信、生命周期、长短命令路径
- [解析管线](explanation/parser-pipeline.md) — 六层管线、事件模型、上下文富化
- [设计原则](explanation/design-principles.md) — token 克制主义、语义 > 压缩、不做清单
- [接入模型](explanation/integration-model.md) — MCP 原生 + bash 代理、零残余哲学
- [安全模型](explanation/security-model.md) — 威胁模型、命令过滤、路径沙箱、审计
- [测试体系](explanation/testing.md) — 分层测试策略与质量门禁

## 营销素材（Marketing · 对开发者公开声明）

诚实首发的对外承诺——所有数字可由 `scripts/measure-savings.sh` 复现：

- [性能声明](marketing/CLAIMS.md) — 组件化质量指标、引用守则与复现 checklist
- [常见问题](marketing/FAQ.md) — 安装、性能、集成、安全、故障排除
- [vs RTK / Headroom](marketing/benchmarks.md) — 架构定位对比（非 head-to-head）

## 决策记录（Decisions）

- [ADR 索引](decisions/README.md) — 已采纳决策与讨论中的草案（从 2026-08 起补齐）

## 产品与设计文档（存档，未随技术文档重写）

- [战略路线图与北极星](ROADMAP-STRATEGY.md)
- [发布清单](release-checklist.md)
- [归档文档](archive/README.md)（历史计划与规格设计；v0.1 产品思考档案已移至本地私有目录）
- [指标证据快照](evidence/2026-08-03.json)（`scripts/evidence_snapshot.sh` 生成，带 git commit 溯源）
