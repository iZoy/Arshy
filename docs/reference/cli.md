# CLI reference

## MCP

`arshy mcp serve` starts the stdio MCP server. `arshy mcp config` prints a
portable configuration snippet; use it in any MCP-compatible client.

## Execution and daemon

```text
arshy run <command> [--cwd <path>] [--format pretty|json] [--errors-only]
arshy daemon start|stop|status|stats|restart
arshy stats [--format pretty|json]
arshy analyze [--format pretty|json]
arshy benchmark
arshy doctor
arshy parser list|reload
arshy config get <key>
arshy self-update [--dest <dir>]
```

`run` is the execution path. It returns structured events and exact output
counters; it never includes quality analytics. `stats`, `analyze`, and
`benchmark` are explicit analytics surfaces and may return component-only
`quality-v1` measurements.

There are no `setup`, `install --agent`, `uninstall --agent`, `hook`, or
agent-specific commands. Installation and MCP registration are intentionally
separate: the installer places binaries, while the client owns its MCP config.

## Exit status

Zero means the requested operation completed. Non-zero means invalid input,
daemon/IPC failure, command failure, or a failed security guard. JSON output is
stable enough for automation but should be validated against the versioned IPC
types in `src/ipc/mod.rs`.
