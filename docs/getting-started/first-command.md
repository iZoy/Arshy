# 第一条命令

Arshy 安装后，agent 自动通过 `arshy_exec` MCP 工具执行所有 shell 命令。你不需要手动选择模式 — `mode: "auto"` 是默认值。

## 短命令

在 Claude Code 中直接让 agent 执行任何命令：

```
> 用 arshy 执行 ls -la
```

Agent 调用 `arshy_exec(action:"run", command:"ls -la")`，短命令瞬间返回原始文本：

```
total 48
drwxr-xr-x  5 user  staff  160 May 17 14:00 .
drwxr-xr-x 12 user  staff  384 May 17 13:00 ..
-rw-r--r--  1 user  staff  256 May 17 12:00 main.rs
```

## 长命令

```
> 用 arshy 执行 cargo test
```

Agent 调用 `arshy_exec(action:"run", command:"cargo test")`。如果命令 2 秒内完成，直接返回结构化结果：

```json
{
  "task_id": "b80c02fa...",
  "status": "completed",
  "exit_code": 0,
  "duration_ms": 7350,
  "event_count": 334
}
```

如果命令运行时间超过 2 秒，返回 `{status:"running", task_id:"..."}`。Agent 自动调用 `arshy_exec(action:"subscribe", task_id:"...")` 等待完成。

## 查询结构化事件

```
> 用 arshy_query 查看 cargo test 的结果
```

Agent 调用 `arshy_query(task_id:"...")` 获取类型化事件：

```json
{
  "events": [
    {
      "type": "test_result",
      "severity": "info",
      "message": "test exec::tests::auto_short_returns_raw_output ... ok"
    }
  ],
  "total": 334
}
```

可按 `event_type`、`severity`、`code`、`file` 过滤。

## 工作目录

Agent 通过 `arshy_exec(action:"cd", command:"/path/to/project")` 设置会话工作目录，后续所有 `run` 调用继承此路径。

## 你不需要思考的事

- **sync 还是 async？** — `mode: "auto"` 自动判断，你永远不需要手动设置
- **短还是长？** — `is_short_command()` 根据 80+ 条规则自动分类
- **输出格式是什么？** — 20 个内置 parser 自动结构化编译错误、测试结果、lint 警告
- **Bash 还是 arshy？** — arshy 在 MCP initialize 时宣告 "I am your shell"，agent 自然优先使用
