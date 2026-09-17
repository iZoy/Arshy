# CLI reference

## MCP

`arshy mcp serve` starts the stdio MCP server.

`arshy mcp config --format json` prints a client-neutral JSON entry with the
current absolute executable path. `--format command` prints the equivalent
shell-style entry. `--format prompt` prints the versioned, copyable Agent
setup procedure; stdout contains only that Prompt.

## Execution and daemon

```bash
arshy run <command> [--cwd <path>] [--format pretty|json] [--errors-only]
arshy status
arshy daemon start|stop|restart
arshy stats [--format pretty|json]
arshy analyze [--format pretty|json]
arshy benchmark
arshy doctor [--format text|json]
arshy parser list|reload
arshy config get <key>
arshy self-update [--dest <dir>]
```

`run` is the execution path. It returns structured events and exact output
counters; it never includes quality analytics. `stats`, `analyze`, and
`benchmark` are explicit analytics surfaces and may return component-only
`quality-v1` measurements.

`doctor --format json` reports only version, OS/architecture, daemon and MCP
protocol status, and parser count. It runs locally and does not upload data.

The installer still does not edit client configuration, install hooks, or
change PATH. Client configuration and lifecycle remain owned by the client;
the setup Prompt stops on conflicts and reports any manual or restart step.

## Exit status

Zero means the requested operation completed. Non-zero means invalid input,
daemon/IPC failure, command failure, or a failed security guard. JSON output is
stable enough for automation but should be validated against the versioned IPC
types in `src/ipc/mod.rs`.
