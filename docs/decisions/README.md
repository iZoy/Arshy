# 决策记录（Architecture Decision Records）

本目录记录 arshy 的关键产品与架构决策。每个 ADR 遵循固定结构：**背景（Context）→ 决策（Decision）→ 理由（Rationale）→ 后果（Consequences）**，并标注状态与日期。

决策史是项目的宝贵资产——本目录从 2026-08-22 开始补齐（此前只有 `docs/archive/` 中的计划与规格，缺少"最终选择了什么、为什么"的记录）。

| 编号 | 决策 | 状态 | 日期 |
|---|---|---|---|
| [0001](0001-restricted-reference-tables.md) | 受限错误码参考表（替代 HintDb 争议） | 已采纳 | 2026-08-22 |
| [0002](0002-jsonl-retention-and-scale-trigger.md) | JSONL 存储维持 + 规模触发线 | 已采纳 | 2026-08-22 |
| [0003](0003-ai-native-positioning-and-platform.md) | AI-native 定位：默认 agent、保留人类通道、Unix-like only | 已采纳 | 2026-08-22 |
| [0004](0004-replay-idempotency-and-execution-safety.md) | 请求重放幂等 + 并发上限执行 + 空闲退出修复 | 已采纳 | 2026-08-22 |
| [0005](0005-remove-unreleased-compat-layer.md) | 移除未发布兼容层（legacy 工具名 / integrate 别名 / 未知工具报错） | 已采纳 | 2026-08-22 |
| [0006](0006-value-anchor.md) | 价值锚点：注意力编译器 + 本地世界中介 + 社区规则资产（决策 1） | 已采纳 | 2026-08-22 |
| [0007](0007-data-driven-execution-and-on-demand-daemon.md) | 数据驱动执行路径、单一职责 MCP 与事件驱动 daemon | 已采纳 | 2026-08-24 |

## 约定

- 已采纳 = 方向确定、已落地或明确排期；
- 草案 = 正在讨论，未拍板；
- 被推翻的旧决策在相应 ADR 中注明，不删除历史。
