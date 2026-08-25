# ADR-0005: 移除未发布兼容层

- **状态**: 部分被 ADR-0007 取代（2026-08-24）；移除 legacy 与未知工具 fail-closed 仍有效
- **关联决策**:零争议清理（"不需要兼容，我们没有发布"）
- **相关代码**:`src/proxy/handlers.rs`、`src/main.rs`、`docs/reference/cli.md`

## 背景

项目在 v0.1.0 之前未公开发布，因此移除了 legacy MCP 工具名、agent 集成命令和未知工具静默回退。v0.1.0 是首个公开 API，不承诺内部开发版本兼容。

## 决策

- 删除 legacy MCP 工具名映射，只保留 3-tool 模型（`arshy_exec`、`arshy_query`、`arshy_task`）；
- 未知工具从"静默当 run"改为返回 `METHOD_NOT_FOUND` 错误；
- 删除 `arshy integrate`、`arshy setup` 等 agent 专用 CLI，MCP 客户端自行注册 `arshy mcp serve`。

## 理由

1. 未发布的兼容层没有收益、只有维护成本与认知噪声；
2. 未知工具静默执行是错误方向的安全默认——fail-closed 原则；
3. 移除时机窗口只在发布前存在，现在是最低成本时刻。

## 后果

- 正面：MCP 表面更干净、危险默认消除、CLI 表面收敛；
- 负面：未来若需兼容旧客户端需重新引入（记录为已知成本）；
- 风险：无（未发布）。
