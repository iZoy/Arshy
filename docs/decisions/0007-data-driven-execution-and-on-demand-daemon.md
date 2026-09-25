# ADR-0007: 数据驱动执行路径、单一职责 MCP 与事件驱动 daemon

- **状态**: 已采纳（2026-08-24）
- **取代范围**: ADR-0005 的 legacy MCP 兼容层；ADR-0004 的轮询式空闲检查实现
- **相关代码**: `src/daemon/exec/decision.rs`、`src/mcp/instructions.rs`、`src/daemon/store/mod.rs`、`src/daemon/main.rs`

## 背景

硬编码的命令前缀列表会让新增 parser 支持也需要修改 Rust。一个包含大量 action 专属字段的 MCP 工具也难以让客户端稳定使用。周期性 idle/flush 检查会唤醒本可休眠的 daemon；每次启动校验全部历史数据，则会让启动成本随保留数据增长。

## 决策

1. “长/短命令”改为“原始快速路径/结构化路径”。parser registry 是否命中成为结构化路径的资产信号；Rust 只保留与具体生态无关的 shell 生命周期规则和稳定的只读检查工具例外。
2. MCP 改为三个单一职责工具：`arshy_exec` 只执行，`arshy_query` 只查询持久化结构化诊断，`arshy_task` 承担低频 cancel/list/raw。公开表面移除隐式 session `cd`、重复的事件 tail 和阻塞 subscribe。
3. daemon 默认按需拉起，空闲 300 秒退出。空闲退出使用 activity notification + 精确 deadline，Store flush 仅由 dirty notification 唤醒。parser 热更新使用平台原生文件事件，不指定周期 polling。
4. `store.integrity_check` 默认关闭，避免冷启动成本随全部事件历史线性增长；需要审计时显式启用。
5. 冷启动时把上一 daemon 遗留的 `running` 任务恢复为 `failed` 终态；旧子进程及其管道无法跨 daemon 重连，继续保留 running 会永久阻塞空闲退出。

## 理由

- parser TOML 是生态扩展资产，应同时拥有工具检测和结构化执行资格，不能要求第二次修改 Rust 名单；
- “两个工具”不是天然更薄，一个大型条件联合 schema 会增加字段误用和错误 action 的概率；
- 常驻资源优化的重点不是单次 timer 很小，而是让完全空闲状态没有无意义周期唤醒；
- 按需 daemon 的冷启动路径必须避免与历史规模绑定的全量工作。

## 后果

- 新增 parser TOML 后，匹配命令自动进入结构化路径；只读检查命令仍返回原始文本；
- MCP 工具数由 2 变为 3，但高频 `arshy_exec` schema 显著缩小，职责可直接从工具名判断；
- 集成方使用文档所述的三个 MCP 工具契约；
- daemon 空闲期间没有 idle/flush 固定轮询，默认最长驻留约五分钟；
- 完整性审计需要显式开启，正常启动仍会解析 `tasks.jsonl` 以恢复任务索引。
