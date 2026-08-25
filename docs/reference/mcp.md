# MCP reference

Start the client-neutral stdio server with:

```sh
arshy mcp serve
```

`arshy mcp config` prints the equivalent JSON registration. The server reads
one JSON-RPC message per line from stdin and writes responses/notifications to
stdout, forwarding execution to `arshyd` over its Unix socket.

The public surface has three single-purpose tools:

| Tool | Responsibility |
|---|---|
| `arshy_exec` | Execute one command. `command` is required; `cwd`, `mode`, timeout, environment, and parser hint are optional. |
| `arshy_query` | Search persisted structured diagnostic events for one task or across history. Raw log-only lines are excluded. |
| `arshy_task` | Low-frequency lifecycle operations: `cancel`, `list`, and `raw`. |

`cd`, event `tail`, and blocking `subscribe` are intentionally absent from the
MCP surface. Per-call `cwd` avoids hidden session state, diagnostics already
cover the event view, and `mode:"auto"` plus notifications cover normal task
completion. The underlying IPC and CLI may expose additional operator controls;
they are not part of the agent-facing contract.

Analytics remain separate CLI commands and are not injected into execution
responses.
