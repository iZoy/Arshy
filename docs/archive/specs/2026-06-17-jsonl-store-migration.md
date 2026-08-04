# SQLite → JSONL 迁移计划

**日期:** 2026-06-17
**状态:** 执行中

## 目标

用 JSONL 文件替代 SQLite，删除 rusqlite 依赖，简化存储层。

## 文件布局

```
~/.local/share/arshy/store/
  tasks.jsonl          # 每行一个 TaskRecord JSON
  events/
    <task_id>.jsonl    # 每行一个 EventRecord JSON
  versions.json        # 工具版本缓存
```

## 核心设计

- Store 结构体: `dir: PathBuf` + `tasks: Mutex<HashMap<String, TaskRecord>>` + `versions: Mutex<HashMap<...>>`
- 写入: 任务用 atomic rename 全量写 tasks.jsonl；事件 append 到 events/<task_id>.jsonl
- 读取: 内存 HashMap 查任务；读单个事件文件查事件
- 统计: 遍历内存 HashMap 计算
- 迁移: 检测旧 .db 文件，一次性读取迁移到 JSONL

## 实施阶段

1. mod.rs — 新 Store 结构体
2. tasks.rs — 任务 CRUD
3. events.rs — 事件写入/查询
4. versions.rs — 版本缓存
5. schema.rs → stats.rs — 统计聚合
6. prune.rs — 清理
7. migration.rs — SQLite 迁移
8. cleanup — 删除旧代码、rusqlite 依赖

## 关键约束

- 所有现有 Store 方法签名不变
- 所有 370 个测试必须通过
- atomic rename 保证崩溃安全
- serde #[serde(default)] 替代 schema 迁移
