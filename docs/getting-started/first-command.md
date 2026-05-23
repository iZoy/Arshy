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

Agent 调用 `arshy_exec(action:"run", command:"cargo test")`。Smart Sync 等命令完成，返回完整结构化结果：

```json
{
  "status": "completed",
  "exit_code": 0,
  "duration_ms": 7350,
  "event_count": 334,
  "summary": {
    "by_type": {"test_result": 31, "log": 6, "summary": 1},
    "by_severity": {"info": 36, "warning": 2}
  },
  "root_cause": null,
  "events": [
    {"type": "diagnostic", "severity": "error", "code": "E0308", "message": "mismatched types"},
    {"type": "location", "severity": "info", "file": "src/main.rs", "line": 10}
  ]
}
```

**1 次 MCP 调用，拿到全部信息。** 不需要 subscribe，不需要 query，不需要轮询。

## 失败场景

```json
{
  "status": "failed",
  "exit_code": 1,
  "summary": {
    "by_type": {"diagnostic": 1, "location": 1, "log": 9},
    "by_severity": {"error": 3, "info": 8}
  },
  "root_cause": {
    "code": "E0308",
    "message": "mismatched types",
    "location": {"file": "src/main.rs", "line": 10, "column": 5}
  },
  "project_context": {
    "git_diff_stat": " src/main.rs | 3 ++- 1 file changed, 2 insertions(+), 1 deletion(-)"
  }
}
```

Agent 直接读：
- `summary.by_severity.error > 0` → 失败
- `root_cause.code` → E0308
- `root_cause.location` → src/main.rs:10
- `project_context.git_diff_stat` → 最近改了什么

## 格式检测

输出是 JSON/YAML/CSV？arshy 自动识别：

```bash
# JSON 自动解析
echo '{"level":"error","message":"disk full"}'
# → {type: "diagnostic", severity: "error", message: "disk full"}

# YAML 自动解析
kubectl get pods -o yaml
# → 结构化事件（key: value 对）

# CSV/TSV 自动解析
docker ps
# → 结构化事件（表格行）
```

## 工作目录

```
arshy_exec(action:"cd", command:"/path/to/project")
```

后续所有 `run` 调用继承此路径。
