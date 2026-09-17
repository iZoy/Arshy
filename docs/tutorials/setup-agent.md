# MCP client setup

Arshy has one client-neutral integration surface: a standard stdio MCP server.
The user starts the flow explicitly and lets the current Agent use its own
native configuration mechanism.

Run this on the target machine and in the project where the Agent will work:

```bash
arshy mcp config --format prompt
```

Copy the complete output and send it to the current Agent. The Prompt contains
the absolute executable path, so do not reuse it on another machine. The Agent
will inspect its own MCP configuration, detect conflicts, respect existing
project rules, verify the result, and report any required restart or manual
GUI/policy step.

For a manual fallback, use:

```bash
arshy mcp config --format json
```

The client owns its configuration and lifecycle. Restart it or open a new task
after registration; MCP tools are not guaranteed to appear in an already
loaded session.

The canonical Prompt is generated from the implementation. Codex and Claude
Code are release-gate clients; any standard stdio MCP client may work, but its
registration scope and project rules remain owned by that client.
