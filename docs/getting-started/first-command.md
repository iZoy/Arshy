# 第一条命令

Arshy 安装后，agent 自动通过 `arshy_exec` MCP 工具执行所有 shell 命令。`mode: "auto"` 是默认值。

## 短命令

```
> 用 arshy 执行 ls -la
```

Agent 调用 `arshy_exec(action:"run", command:"ls -la")`，瞬间返回原始文本：

```
total 48
drwxr-xr-x  5 user  staff  160 May 17 14:00 .
```

## 长命令（1 次调用，完整结果）

```
> 用 arshy 执行 cargo test
```

Agent 调用 `arshy_exec(action:"run", command:"cargo test")`。auto 模式使用 smart sync — 等命令完成，返回完整结构化结果：

```json
{
  "task_id": "b80c02fa...",
  "status": "completed",
  "exit_code": 0,
  "duration_ms": 7350,
  "event_count": 334,
  "events": [
    { "type": "diagnostic", "severity": "error", "code": "E0308", "message": "mismatched types" },
    { "type": "location", "severity": "info", "file": "src/main.rs", "line": 10 }
  ]
}
```

**1 次 MCP 调用，拿到全部信息。** 不需要 subscribe，不需要 query，不需要轮询。

## 超长命令（>30 秒自动降级）

`docker build`、大型项目构建等命令如果超过 30 秒，auto 模式自动降级为异步：

```json
{
  "task_id": "b8c81ec2...",
  "status": "running"
}
```

对于这些极少数情况，agent 可以调用 `arshy_exec(action:"subscribe", task_id:"...")` 等待完成。

## 工作目录

```
arshy_exec(action:"cd", command:"/path/to/project")
```

后续所有 `run` 调用继承此路径。
