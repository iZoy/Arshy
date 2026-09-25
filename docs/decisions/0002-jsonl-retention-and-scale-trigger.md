# ADR-0002: JSONL 存储维持 + 规模触发线

- **状态**:已采纳（2026-08-22）
- **相关代码**:`src/daemon/store/`

## 背景

JSONL supports append-oriented task and event storage without an external database. Cross-task search scans event files, so its latency can grow with the amount of retained history.

## 决策

维持 JSONL，**不主动迁移数据库**，写入明确触发线：

- 任务数 > **10,000** 或 `arshy query` 跨任务搜索平均延迟 > **200ms** 时，先评估**派生索引**（如 `file → task_id` 倒排、惰性重建），而不是直接迁移 SQLite；
- 迁移必须由测量触发——不做第二次无数据支撑的反转。

## 理由

1. JSONL keeps local storage simple, inspectable, and free of an external database dependency;
2. 触发线把"何时优化"从争论变成可测量条件；
3. 派生索引是中间态，成本远低于数据库迁移，且保留 JSONL 的可调试性。

## 后果

- 正面：存储层保持简单；优化时机由数据决定；
- 负面：在达到触发线之前，跨任务搜索会随规模线性变慢——这是接受的代价；
- 风险：触发线数值是估计值，需在接近时验证。
