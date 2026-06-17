# Phase 0: Feature Usage Instrumentation

**Date:** 2026-06-11
**Goal:** 低耦合计数器 + 用户可见的价值展示 + benchmark 证明价值

---

## 设计原则

1. **低耦合** — 计数器是各模块的内部字段，不改函数签名，不加参数传递
2. **用户可见** — `arshy stats` 的 "Feature Usage" 区域让任何用户一眼看到 arshy 在帮他们做什么
3. **可验证** — benchmark 脚本证明端到端价值，不是理论指标

## 改动清单

### 1. Deduplicator 计数器

`src/daemon/parser/dedup.rs`:
- 加 `total_collapsed: u64` 字段（private）
- `feed()` 返回 None 时 `total_collapsed += 1`
- 暴露 `pub fn collapsed_count(&self) -> u64`
- 不改 `feed()` 签名，不改返回类型

### 2. DB 迁移 v5

`src/daemon/store/schema.rs`:
- `ALTER TABLE tasks ADD COLUMN dedup_collapsed INTEGER DEFAULT 0`
- `ALTER TABLE tasks ADD COLUMN correlated_errors INTEGER DEFAULT 0`

### 3. Task 结构体扩展

`src/ipc/mod.rs` — Task struct 加两个 Option 字段：
- `dedup_collapsed: Option<u64>`
- `correlated_errors: Option<u64>`

### 4. Executor 持久化计数器

`src/daemon/exec/mod.rs`:
- 事件循环结束时，`dedup.collapsed_count()` 写入 tasks 表
- git correlation 结果的 `correlated_count` 写入 tasks 表
- 新增 `update_task_counters()` 方法

### 5. Stats 聚合

`src/daemon/store/schema.rs` — `get_stats()` 新增：
- `dedup_collapsed: Option<u64>` — `SELECT SUM(dedup_collapsed) FROM tasks`
- `correlated_errors: Option<u64>` — `SELECT SUM(correlated_errors) FROM tasks`
- `per_parser_usage: Option<Vec<ParserUsage>>` — `SELECT parser_name, COUNT(*) FROM tasks WHERE parser_name IS NOT NULL GROUP BY parser_name ORDER BY count DESC`

### 6. StatsResponse 扩展

`src/ipc/mod.rs`:
- `dedup_collapsed: Option<u64>`
- `correlated_errors: Option<u64>`
- `per_parser_usage: Option<Vec<ParserCount>>`

### 7. CLI 渲染

`src/cli/render.rs` — `render_stats()` 新增 "Feature Usage" 区域：
```
Feature Usage
  Events deduplicated: 42 events collapsed (saved agent reads)
  Git correlation:     8 errors linked to recent changes
  Top parsers:         tsc (12), cargo (8), eslint (5)
```

### 8. Benchmark 增强

`scripts/benchmark.sh` 扩展：
- 现有 parser accuracy 不变
- 新增 "Feature Value" 区域：统计 fixture 中有多少事件有 location、context、hint
- 输出格式：`Fields/event | Dedup potential | Context coverage | Hint coverage`

---

## 不改的

- 不改 `Deduplicator::feed()` 签名
- 不加新模块、新文件
- 不改 parser 定义
- 不改安全模块
- 不加新的 CLI 命令
