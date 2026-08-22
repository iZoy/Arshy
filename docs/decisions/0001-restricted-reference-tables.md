# ADR-0001: 受限错误码参考表

- **状态**:已采纳（2026-08-22）
- **关联决策**:审计分歧 2
- **相关代码**:`src/daemon/reference/`、`reference/builtin/*.toml`、`docs/reference/reference-codes.md`

## 背景

项目曾存在 HintDb（错误码 → cause/fix/retry 建议表），后整体移除（`parsers/errors/*.toml` 删除、`TaskEvent.hint` 保留为 null 兼容占位），哲学声明"建议是 LLM 的职责"。但：

1. Phase 4 执行层转向计划（2026-07-29 草案）工作项 4.1 要求"深耕层补齐错误码 hint 覆盖"，与移除声明直接冲突；
2. 同一份计划内部自相矛盾（4.1 要 hint，S2 又确认移除）；
3. 代码残留 `EventHint`/`RetryHint` 类型、`events_with_hint` 指标、`_tool_name` 参数等已删功能的尸体。

## 决策

采用**受限参考表**方案：

- 只解释**非显而易见**错误码/退出码的**含义**（docker 125/126/127/137、kubectl/aws 退出码），条目仅含 `code` + `message`（含义）+ `source`（验证链接），**不含 cause/fix/retry**；
- 只通过 `task/query` **按需返回**（命中事件附加 `reference` 数组），**永不内联进存储的事件流**；`TaskEvent.hint` 保持 null 占位；
- 表本身是 TOML 数据文件：内置表 `reference/builtin/*.toml` 编译时嵌入，用户表 `~/.arshy/reference/*.toml` 同名 `meta.name` 覆盖；
- 移除 `events_with_hint` 死指标（C 方案下存储事件永不携带 hint，该指标恒为 0）；`EventHint`/`RetryHint` 作为协议占位保留。

## 理由

1. "错误码参考"与"cause/fix 建议"是两件事：项目移除的是**建议**，Phase 4 想要的是**非显而易见代码的查表**——拆开后两者不冲突；
2. 护栏（只查表、不进事件流、数据驱动、带验证链接）复用项目的克制主义与数据驱动哲学；
3. docker/kubectl/aws 这类退出码不是 LLM 常识，查表有真实价值；编译错误码（E0425 等）消息自解释，明确不入表。

## 后果

- 正面：拿到 Phase 4 想要的查表价值，同时保持"不合成建议"的哲学自洽；死指标清理；
- 负面：查询路径多一次查表开销（HashMap 查找，可忽略）；参考表内容需要维护与验证来源；
- 风险：条目质量依赖人工维护——通过"仅含义 + 来源链接"约束，把过期误导风险降到最低；
- 后续：热重载（当前守护进程启动时加载）；工具维度的查找（当前按 code 全局查找）。
