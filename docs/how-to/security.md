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

Command filtering is a best-effort guardrail. It recognizes common quote and
option spellings for destructive recursive removal and common destructive Git
worktree operations such as forced `git clean`, `git reset --hard`,
`git switch --discard-changes`, stash deletion, and path restoration through
`checkout` or `restore`. Dry-run cleanup and staged-only restore forms remain
usable. Broad `find` deletion from protected roots, including `find . -delete`
and `find . -exec rm ...`, is also rejected; scoped cleanup under paths such as
`./build` remains available. The filter rejects dynamic `eval` arguments and
nested `-c` scripts whose command name is generated at runtime. It also checks
variable-expanded command names when paired with recognized destructive `rm`
or Git arguments. It does not fully evaluate shell expansions, functions, or
arbitrary shell code. It inspects common wrappers such as `env -S`, BusyBox,
and Toybox, but this coverage is not exhaustive. Do not use it as a sandbox
for hostile commands.

`security.blocked_patterns` replaces the default regex pattern list when set.
This lets an operator customize regex matching, but does not disable the
daemon's separate command-structure guardrails for destructive `rm`, Git, and
`find` forms. Treat the regex list as an additional configurable filter, not
as the switch that enables or disables those structural checks.

Diagnostic enrichment reads up to three neighboring lines only from files
whose canonical paths remain inside the requested working directory. Absolute
or relative diagnostic locations outside that directory, including symlinks
that escape it, keep their parsed locations but receive no automatic source
context.

`security.access_level = "read-only"` limits execution access: IPC rejects
`task/run` and `task/kill`, while diagnostic reads remain available. It does
not make the task store read-only. Same-user local clients can still use daemon
management methods such as `daemon/prune` (which deletes retained history) and
`daemon/shutdown`; all same-UID processes are inside the socket trust boundary.

Run `arshy doctor` after changing configuration and inspect the audit log when
debugging a rejected command.

On Unix, the daemon sets the store, `raw/`, and `events/` directories to mode
`0700` and store files to `0600` when it opens the store. It also tightens
existing store permissions and refuses symlinked store entries. This protects
stored commands and output from other local user IDs when the store path's
parent directories permit access. Processes running as the same user remain
inside the trust boundary and can access that user's data.
