# 决策记录（Architecture Decision Records）

本目录记录 arshy 的关键架构与实现决策，供贡献者了解当前设计及其取舍。每个 ADR 遵循固定结构：**背景（Context）→ 决策（Decision）→ 理由（Rationale）→ 后果（Consequences）**，并标注状态与日期。

这些记录解释了主要实现选择及其取舍；细节可能随代码演进而变化，请以当前代码和测试为准。

| 编号 | 决策 | 状态 | 日期 |
|---|---|---|---|
| [0001](0001-restricted-reference-tables.md) | 受限错误码参考表（替代 HintDb 争议） | 已采纳 | 2026-08-22 |
| [0002](0002-jsonl-retention-and-scale-trigger.md) | JSONL 存储维持 + 规模触发线 | 已采纳 | 2026-08-22 |
| [0003](0003-ai-native-positioning-and-platform.md) | AI-native 定位：默认 agent、保留人类通道、Unix-like only | 已采纳 | 2026-08-22 |
| [0004](0004-replay-idempotency-and-execution-safety.md) | 请求重放幂等 + 并发上限执行 + 空闲退出修复 | 已采纳 | 2026-08-22 |
| [0005](0005-remove-unreleased-compat-layer.md) | 拒绝旧版与未知 MCP 工具 | 已采纳 | 2026-08-22 |
| [0007](0007-data-driven-execution-and-on-demand-daemon.md) | 数据驱动执行路径、单一职责 MCP 与事件驱动 daemon | 已采纳 | 2026-08-24 |
| [0008](0008-prompt-first-agent-onboarding.md) | 通用 MCP 核心 + Agent 自配置 Prompt | 已采纳 | 2026-08-25 |

## 约定

- 已采纳 = 方向确定、已落地或明确排期；
- 草案 = 正在讨论，未拍板；
- 被推翻的旧决策在相应 ADR 中注明，不删除历史。
