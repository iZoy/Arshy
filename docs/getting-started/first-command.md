# 第一条命令

## 启动 daemon

```bash
arshy daemon start
```

输出：`daemon started (pid xxxxx)`

## 执行命令

```bash
arshy run "echo hello world"
```

短命令（< 2 秒预估）直接返回原文：

```json
{
  "task_id": "uuid",
  "status": "completed",
  "exit_code": 0,
  "raw_output": "hello world\n",
  "short_command": true
}
```

## 异步执行

```bash
arshy run "sleep 5 && echo done" --mode async
```

立即返回 task_id，后台执行。查看结果：

```bash
arshy tail <task_id>
```

## 查看任务列表

```bash
arshy list
```

## 查询结构化事件

```bash
arshy query <task_id>
```

## 停止 daemon

```bash
arshy daemon stop
```
