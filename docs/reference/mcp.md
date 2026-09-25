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

After a daemon connection failure, the proxy may reconnect and retry read-only
`arshy_query` calls and `arshy_task` `list`/`raw` calls. It does not retry
`arshy_exec`, cancellation, or other potentially state-changing calls. If the
connection is lost during execution, the result is unknown; inspect task
history before deciding whether to run the command again.

The daemon deduplicates replayed executions by proxy-session request key and
the execution arguments. Cached results live in daemon memory for up to 120
seconds, with a 256-entry limit and an approximate 64 MiB response budget; a
single oversized result can exceed that budget. A daemon restart clears this
cache. This protects the reconnect-and-replay path, but it is not durable
exactly-once execution.

`arshy_exec` runs the command string with `sh -c` in a non-interactive child.
The child reads from null stdin (immediate EOF); stdout and stderr are captured
through separate pipes. No pseudo-terminal is allocated, so interactive prompts,
TTY-dependent behavior, and terminal color should not be expected. `TERM=dumb`
is set. A login shell is probed only to supplement `PATH`; the command itself
does not run as a login or interactive shell, and other profile settings are
not loaded. Shell pipelines and redirections are supported by `sh -c`. Relative
ordering between stdout and stderr lines is not guaranteed.

Output is bounded by `daemon.max_output_bytes` (10 MiB by default), with a
64 KiB per-line rendering cap. Arshy replaces invalid UTF-8 and adds visible
truncation markers; `raw_output_bytes` counts source bytes read, including
drained bytes beyond the capture cap, and is not a byte-exact replay size.
MCP execution responses inline at most 16 KiB, keeping the beginning and end
of longer output. The response includes a task handle and a short retrieval
instruction; use `arshy_task(action:"raw", task_id:"…", lines:0)` for all
captured lines.
Structured-task output is retained up to the capture limit. A truncated fast
path persists its captured prefix and a task handle; bytes beyond the limit
were already discarded. In `auto` mode, 60 seconds is only the initial
wait before returning a still-running task; execution continues until its
configured timeout or completion.

`cd`, event `tail`, and blocking `subscribe` are intentionally absent from the
MCP surface. Per-call `cwd` avoids hidden session state, diagnostics already
cover the event view, and `mode:"auto"` plus notifications cover normal task
completion. The underlying IPC and CLI may expose additional operator controls;
they are not part of the agent-facing contract.

Notifications are best-effort progress hints. Bounded daemon and proxy queues
can drop notifications under load or during disconnects; an overflow notice
may report skipped messages. Use `arshy_query` for structured diagnostics and
`arshy_task` `list`/`raw` for persisted task state and captured output. A task
ID is returned only while execution is running or when inline output is
incomplete; complete synchronous results omit it.

Analytics remain separate CLI commands and are not injected into execution
responses.

Tool definitions publish outputSchema. Tool calls return machine-readable data
under the standard structuredContent field and a text fallback under content.
Execution structuredContent always includes `status` and `exit_code`; exit_code
is null while a task is running. Successful text contains command output (or
`✓` when empty). Failed text preserves command output; structured failure data
contains only known severity, location, and code. A needed `task_id` appears in
both text and structuredContent for text-only clients.
