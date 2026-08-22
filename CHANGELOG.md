# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## 版本说明（重要）

本仓库的 `v0.0.1` 是 **首个公开发布的开源版本**（2026-08-23，commit `08e0ee8`）。在此之前的内部开发阶段仅用于团队追踪，不对外发布：

| 阶段 | 范围 | 说明 |
|---|---|---|
| **v0.0.1（开源首发）** | 2026-08-23 | 首个开源版本：`docs/marketing/CLAIMS.md` 中所有数字校准于此版本 |
| dev-0.0.0-α | 2026-08 早期 | 决策落地 + Phase A/B 诚实性补丁 + 跨生态 e2e + 跨主流生态测量 |
| dev-0.0.0-α-prv | 2026-08-04 | 内部 dogfooding 验证（219 tasks, 758 dogfood events, Rust 偏置明显） |
| dev-0.0.0-α-dev | 2026-08-03 | 决策记录 ADR-0001~0006 起草；指标证据快照基础设施 |
| dev-0.0.0-α-init | 2026-08-03 之前 | 双二进制 + 37 内置 parser + MCP server + 安全沙箱的初始骨架 |

**不要引用 dev-* 阶段的任何数字**——它们没有诚实声明、可能包含 Rust 偏置样本、未对外公布。
引用 `v0.0.1` 数据请同时附 [`docs/marketing/CLAIMS.md`](docs/marketing/CLAIMS.md) 的诚实边界声明。

---

## [v0.0.1] - 2026-08-23

**首个开源版本（Honest First Release）**——所有营销数字按此版本校准。

### 营销与文档
- **新增** `docs/marketing/CLAIMS.md`——开发者面向的性能与能力声明，诚实边界 + 引用守则
- **新增** `docs/marketing/FAQ.md`——安装、性能、集成、安全、故障排除
- **新增** `docs/marketing/benchmarks.md`——vs RTK / Headroom 定位对比（架构层面，非 head-to-head）
- **新增** `docs/reference/metrics.md`——所有 `daemon/stats` / `daemon/analyze` / `benchmark` 字段含义

### Phase A：Token 节省诚实性
- **新增** `StatsResponse.savings_basis` 字段（`measured` / `estimated` / `none`）
- **新增** `savings_fallback_task_count`（10% 兜底命中次数）
- **新增** `tracing::warn!` 当兜底命中——运维可 grep
- **撤回** 用户面 `TOKEN EFFICIENCY` box——savings 仅在 `--format json` 可见
- **新增** 3 单元 + 4 e2e 测试固定新契约

### Phase B：跨主流生态覆盖（消除 Rust 偏置）
- **新增** 11 个跨主流生态 fixture（npm/tsc/jest/vitest/go/docker/pnpm/pip/ruff/uv/python）
- **新增** 5 个跨生态 e2e 真实命令测试（python traceback / go build / node thrown / rustc / metrics snapshot）
- **新增** `scripts/measure-savings.sh`——按生态分列节省率 + 复现脚本

### Bug 修复
- **修复** `response.error_count` 与 events 数组不一致——直接从 events_json 计算
- **修复** `cwd=None` 时 `compute_enhanced_project_context` 泄露 daemon 自身 git diff stat
- **修复** `calculate_agent_delivered_bytes` fixed-cost 50 字节对小输出不友好
- **修复** bash-proxy 与 CLI run 不传 cwd 默认值

### 测试
- **新增** 8 个单元测试覆盖 `calculate_agent_delivered_bytes` boundary cases
- **新增** 1 个 e2e 测试 `e2e_cli_default_cwd_injected_when_missing`
- **结果**：509 lib + 25 integration + 37 fixture tests pass；fmt + clippy clean

### 实测（per-ecosystem，savings_basis: measured）

| 生态 | raw bytes | delivered bytes | **节省率** |
|---|---|---|---|
| rustc | 333 | 67 | **+79.9%** |
| node  | 770 | 162 | **+79.0%** |
| python | 293 | 99 | **+66.2%** |
| go    | 158 | 73 | **+53.8%** |
| npm   | 268 | 134 | **+50.0%** |
| cargo (sh-c) | 46 | 25 | **+45.7%** |
| **聚合** | **1,868** | **560** | **+70.0%** |

> 复现：`scripts/measure-savings.sh`
> 证据：`docs/evidence/2026-08-23.json`

---

## 内部开发阶段（dev-*，不对外发布）

> **以下阶段不发布，不应被引用**。保留在 changelog 中是为团队追踪与 git 历史连续性。

### dev-0.0.0-α（2026-08 早期）

诚实性补丁与跨生态测量：
- savings_basis 字段（先于开源版本定型）
- 跨生态 e2e 测试套件
- measure-savings 脚本初版
- error_count 与 events 一致性 bug 修复

### dev-0.0.0-α-prv（2026-08-04）

初始 dogfooding 验证：
- 219 tasks, 758 dogfood events（**Rust 偏置 ~90%**——不应作为代表性数据）
- `docs/evidence/2026-08-03.json` 校准此阶段（噪声率 28.2%）

### dev-0.0.0-α-dev（2026-08-03）

决策记录 ADR-0001~0006 起草：
- 受限错误码参考表
- JSONL 存储规模触发线
- AI-native 定位与 Unix-only 决策
- 重放幂等与执行安全
- 移除未发布兼容层
- 价值锚点（注意力编译器 + 本地世界中介 + 社区规则资产）

### dev-0.0.0-α-init（2026-08-03 之前）

初始骨架：
- 双二进制架构：`arshy`（CLI + MCP proxy）+ `arshyd`（daemon）
- 37 个内置 parser（TOML 定义）
- 6 层解析管道
- MCP server 集成
- 安全沙箱（命令过滤 + 路径沙箱 + 审计日志）
- 按需自启 + 空闲退出
