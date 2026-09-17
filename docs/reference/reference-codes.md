# 错误码参考表（Reference Tables）

错误码参考表回答一个窄问题：**这个退出码/错误码是什么意思**。它刻意不回答"怎么修"——修复是 LLM 的职责（见 [设计原则](../explanation/design-principles.md) 的"不合成 cause/fix 建议"）。

## 何时有用

工具输出里的**非显而易见**退出码：docker 的 `125`/`126`/`127`/`137`、kubectl/aws 的退出码。编译器错误码（如 `E0425`）的消息本身已经说明含义，不需要查表。

## 获取方式

- 通过 `arshy_query` 按需返回：查询结果中，带 `code` 字段且命中参考表的事件会附加 `reference` 数组（每个条目含 `tool`/`code`/`message`/`source`）；
- 查找按**任务所属工具**收窄：查询会取任务的 `parser_name` 作为工具维度（跨任务搜索按每条事件的 `task_id` 回查），docker 130 与 aws 130 不会混淆；工具未知时返回 `None`；
- **永不内联**进存储的事件流；`TaskEvent.hint` 保持 null 占位。

## 数据文件

```
reference/builtin/<tool>.toml     # 内置表（编译时嵌入二进制）
~/.arshy/reference/<tool>.toml    # 用户表，同名 meta.name 覆盖内置
```

TOML schema：

```toml
[meta]
name = "docker"
description = "Docker CLI / docker compose exit codes"

[[entry]]
code = "125"
message = "Docker daemon error — the daemon itself failed (e.g. not running or crashed)"
source = "https://docs.docker.com/reference/cli/docker/container/run/#exit-status"
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `meta.name` | 是 | 工具名，也是覆盖/去重的键 |
| `meta.description` | 否 | 描述 |
| `entry.code` | 是 | 退出码/错误码（字符串） |
| `entry.message` | 是 | 该代码的**含义**——参考数据，不是建议 |
| `entry.source` | 否 | 验证链接 |

## 规则

- 只写"代码是什么"，不写"应该怎么办"；
- 添加条目是数据变更，不需要改 Rust；
- **热重载**：内置表编译进二进制；用户表（`~/.arshy/reference/*.toml`）变更会自动热重载（文件监听，`arshy parser reload` 也会同时刷新参考表）。

## 测试

`cargo test reference` 验证内置表加载与未知 code 的查找语义（reference 模块测试位于 library target）。
