# Integration model

Arshy has one public integration boundary: a generic MCP stdio server.

```text
MCP client ──stdio JSON-RPC──> arshy mcp serve ──UDS──> arshyd
                                                     │
                                                     ├─ filter + cwd guard
                                                     ├─ PTY execution
                                                     ├─ parser + enrichment
                                                     └─ JSONL store / events
```

The installer places binaries only. The MCP client owns registration and
removal. This keeps the surface thin and makes support ecosystem-independent:
any client that can launch an MCP stdio server can use Arshy.

The execution path returns structured events and exact byte counters. The
analytics path (`stats`, `analyze`, `benchmark`) is separate and opt-in, so a
normal command call never pays for or receives an efficiency score.
