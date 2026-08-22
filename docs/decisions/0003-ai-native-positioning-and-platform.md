# ADR-0003: AI-native 定位：默认 agent、保留人类通道、Unix-like only

- **状态**:已采纳（2026-08-22）
- **关联决策**:审计分歧 5、6
- **相关代码**:`src/cli/tasks.rs`、`README.md`、`docs/tutorials/install.md`

## 背景

arshy 的 CLI 默认输出 JSON（agent-first），但 `--format pretty` 曾调错渲染函数（`render_stats` 渲染 run 结果），且 `auto` 从未做 TTY 判定、名不副实。平台方面，全项目依赖 Unix 能力（UDS、进程组信号、`sh -c`），但文档残留 Windows/WSL 字样，边界不诚实。

## 决策

1. **AI-native 是唯一默认**：`arshy run` 默认输出 JSON；`--format pretty` 保留给显式的人类观察，`auto` 仅在 stdout 是真实终端时输出人类可读文本（`render_run_text`），否则 JSON；
2. 人类不是目标用户，但**功能保留**（pretty / stats / benchmark / analyze），不删除；
3. **平台 Unix-like only**：仅支持 macOS / Linux；Windows 原生不支持，暂无 WSL 支持计划（WSL2 内运行属于 Linux 环境，按 Linux 对待）。文档同步修正。

## 理由

1. 工具面的每一处默认值都是产品定位的声明——默认给 agent 最优形态，人类需要时显式索取；
2. `auto` 的 TTY 判定兑现其文档语义，且不影响任何 agent 路径（agent 无 TTY）；
3. 平台边界诚实：宣称不支持的平台，比含糊承诺更可信；Windows 原生支持接近重写执行层，0.1 阶段是战略失误。

## 后果

- 正面：定位清晰、行为可预期、文档与实现一致；
- 负面：放弃 Windows 用户市场（明确接受）；
- 风险：若未来需要 Windows，UDS/信号/`sh -c` 全部要重设计——作为已知约束记录，不设预期。
