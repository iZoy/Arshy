# MCP reference

Start the client-neutral stdio server with:

```bash
arshy mcp serve
```

`arshy mcp config --format json` prints the client-neutral JSON registration;
`--format command` prints its shell-style form and `--format prompt` emits the
versioned Agent-guided setup procedure. The server reads one JSON-RPC message
per line from stdin and writes responses/notifications to stdout, forwarding
execution to `arshyd` over its Unix socket.

The public surface has three single-purpose tools:

| Tool | Responsibility |
|---|---|
| `arshy_exec` | Execute one command. `command` is required; `cwd`, `mode`, timeout, and environment are optional. |
| `arshy_query` | Search persisted structured diagnostic events for one task or across history. Raw log-only lines are excluded. |
| `arshy_task` | Low-frequency lifecycle operations: `cancel`, `list`, and `raw`. |

`cd`, event `tail`, and blocking `subscribe` are intentionally absent from the
MCP surface. Per-call `cwd` avoids hidden session state, diagnostics already
cover the event view, and `mode:"auto"` plus notifications cover normal task
completion. The underlying IPC and CLI may expose additional operator controls;
they are not part of the agent-facing contract.

Analytics remain separate CLI commands and are not injected into execution
responses.

Tool definitions publish outputSchema. Tool calls return machine-readable data
under the standard structuredContent field and a text fallback under content.
Execution always includes task_id, status, and exit_code; exit_code is null
while a task is running. primary_diagnostic is representative evidence selected
from the complete error set and is not a causal claim.
