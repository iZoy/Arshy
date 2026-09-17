# Integrate an MCP client

Use the client-neutral setup flow:

```bash
arshy mcp config --format prompt
```

Copy the complete output to the Agent running in the target project. This is
the only guided setup action Arshy asks the user to perform. The Agent owns
client configuration, chooses the native scope, and decides whether existing
project rules need an update.

The Prompt requires exact command/argument matching, conflict detection,
minimal changes, verification, explicit fallback reporting, and a restart/new
task notice. It does not force a particular client, configuration file,
project-rule filename, or Arshy-specific marker.

For manual setup, use:

```bash
arshy mcp config --format json
```

The client owns the MCP registration and lifecycle. Arshy does not install
hooks, shell shims, or PATH changes. The complete canonical Prompt is kept in
the [README](../../README.md).
