# Security model

Arshy protects the local execution boundary with command filtering, optional
working-directory containment, Unix-socket peer validation, resource limits,
and audit logging. These controls address accidental or obviously dangerous
agent commands; they are not a substitute for an OS or container sandbox.
The command filter handles common quote and option forms for destructive
recursive removal and common destructive Git worktree operations, including
forced `git clean`, `git reset --hard`, `git switch --discard-changes`, stash
deletion, path restoration through `checkout` or `restore`, and broad `find`
deletion from protected roots such as the current directory. Scoped cleanup
under a path such as `./build` remains available. It also rejects dynamic
`eval` arguments and nested `-c` scripts whose command name is generated at
runtime, and checks variable-expanded command names paired with recognized
destructive `rm` or Git arguments. It does not fully evaluate shell expansion,
function definitions, or arbitrary code. Treat it as an accidental-command
guardrail.

`security.allowed_cwds` is the only directory policy. When configured, Arshy
canonicalizes the requested cwd and requires it to be inside an allowed root.
An empty list is permissive. The check is fail-closed for invalid paths and
does not restrict file reads, network access, child processes, or credentials.

The short-command optimization skips persistence and parsing work only. It
does not skip filtering, cwd validation, peer checks, limits, or audit logs.
Use a dedicated OS-level sandbox when the threat model includes hostile code
or credentials.

On Unix, the daemon stores task history, events, raw output, and audit data in
owner-only directories (`0700`) and files (`0600`). It applies these modes when
opening the store and refuses symlinked store entries. This limits access by
other local user IDs when parent directories are traversable; same-user
processes remain trusted and can read the data.
