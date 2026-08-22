# ADR-0005: 移除未发布兼容层

- **状态**:已采纳（2026-08-22）
- **关联决策**:零争议清理（"不需要兼容，我们没有发布"）
- **相关代码**:`src/proxy/handlers.rs`、`src/main.rs`、`docs/reference/cli.md`

## 背景

项目未发布（v0.0.1，无真实用户），但代码中保留了多种兼容层：legacy MCP 工具名（`arshy_run`/`arshy_list`/`arshy_kill`/`arshy_tail`）、`arshy integrate` CLI 别名、未知工具静默回退到 run。其中"未知工具默认当 run 执行"意味着拼错工具名会**真的执行命令**，属于危险默认。

## 决策

- 删除 legacy MCP 工具名映射，只保留 2-tool 模型（`arshy_exec` + `arshy_query`）；
- 未知工具从"静默当 run"改为返回 `METHOD_NOT_FOUND` 错误；
- 删除 `arshy integrate` CLI 别名（保留 `setup`），同步更新文档与 doctor 建议文案。

## 理由

1. 未发布的兼容层没有收益、只有维护成本与认知噪声；
2. 未知工具静默执行是错误方向的安全默认——fail-closed 原则；
3. 移除时机窗口只在发布前存在，现在是最低成本时刻。

## 后果

- 正面：MCP 表面更干净、危险默认消除、CLI 表面收敛；
- 负面：未来若需兼容旧客户端需重新引入（记录为已知成本）；
- 风险：无（未发布）。
