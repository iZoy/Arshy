# Security and the directory guard

Arshy is a local command runner, not a container or operating-system sandbox.
It intentionally runs commands as the current user. Review that boundary
before enabling it for an agent.

The controls are:

1. command filtering for dangerous command forms;
2. optional `security.allowed_cwds` validation on every request;
3. Unix-socket peer checks;
4. an append-only audit log for command filtering and execution decisions.

Example:

```toml
[security]
allowed_cwds = ["~/src", "/tmp/arshy-work"]
```

An empty list means no directory restriction. A non-empty list is fail-closed:
the requested `cwd` must resolve inside one of the roots. This protects the
working-directory boundary only; it does not restrict reads, writes, network
access, child processes, or credentials. Use OS-level isolation when those
properties are required.

Run `arshy doctor` after changing configuration and inspect the audit log when
debugging a rejected command.
