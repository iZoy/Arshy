# Integration model

Arshy has one public integration boundary: a generic MCP stdio server.

```text
MCP client ──stdio JSON-RPC──> arshy mcp serve ──UDS──> arshyd
                                                     │
                                                     ├─ filter + cwd guard
                                                     ├─ sh -c + separate stdout/stderr pipes
                                                     ├─ parser + enrichment
                                                     └─ JSONL store / events
```

The installer places binaries only. Any MCP client can use the generic
`arshy mcp serve` entry. `arshy mcp config --format prompt` provides a
client-neutral procedure that asks the current Agent to use its native
registration and, when supported, make only the project-rule changes allowed by
the current project policy. It does not introduce hooks or shell shims.

The execution path returns structured events and source-byte counters. The
analytics path (`stats`, `analyze`, `benchmark`) is separate and opt-in, so a
normal command call never pays for or receives an efficiency score.

Commands run non-interactively through `sh -c`, with null stdin and separate
stdout/stderr pipes. Arshy does not allocate a PTY; tools that require prompts
or TTY behavior are not supported by this execution contract.
