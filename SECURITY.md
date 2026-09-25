# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in arshy, please report it privately to the maintainers. Do NOT open a public issue.

Email: security@izoy.org (or open a private security advisory on GitHub)

We aim to respond within 48 hours and resolve confirmed issues within 7 days.

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Security Model

Arshy executes shell commands with the current user's privileges. It is not an
OS or container sandbox. Its security controls include:

1. **Command Filtering** — Blocked regex patterns prevent known-dangerous commands (`rm -rf /`, `dd if=`, etc.)
2. **Working-directory Guard** — Optionally require the command cwd to be under an allowed root
3. **Audit Logging** — All command execution is logged with timestamps, exit codes, and cwd
4. **Read-only Mode** — `access_level = "read-only"` prevents any command execution

`read-only` here is an execution permission: it rejects `task/run` and
`task/kill`, while allowing diagnostic reads. It is not a read-only data-store
permission. Same-user daemon management calls such as `daemon/prune` and
`daemon/shutdown` remain available; prune can delete retained task history.
The local socket trusts processes running under the same user ID.

### Known Limitations

- `security.allowed_cwds` validates only the starting working directory. It
  does not prevent a command from reading or writing other paths.
- The command filter is a best-effort guardrail, not a shell sandbox. It rejects dynamic `eval` arguments and nested `-c` scripts with a runtime-generated command name, and checks variable-expanded names paired with recognized destructive `rm` or Git arguments. It does not fully evaluate shell expansions, functions, or arbitrary code. Use OS-level isolation for hostile code or credentials.
- Automatic diagnostic source context reads only canonical files inside the
  command's working directory. Locations outside it, including symlink escapes,
  remain in the diagnostic but do not cause an automatic file read.
- The daemon runs with the user's privileges. Do not expose the socket to untrusted processes.

## Best Practices

- Run arshy with the least-privileged user account
- Enable audit logging for production deployments
- Use `security.allowed_cwds` as a cwd policy and an OS/container sandbox
  when filesystem isolation is required
- Keep the blocked patterns list updated
- Review audit logs regularly for suspicious activity
