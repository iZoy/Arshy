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

Arshy executes shell commands on the host system. Its security model includes:

1. **Command Filtering** — Blocked regex patterns prevent known-dangerous commands (`rm -rf /`, `dd if=`, etc.)
2. **Sandbox Paths** — Restrict filesystem access to whitelisted directories
3. **Audit Logging** — All command execution is logged with timestamps, exit codes, and cwd
4. **Read-only Mode** — `access_level = "read-only"` prevents any command execution

### Known Limitations

- Sandbox mode `"process"` and `"container"` are reserved but not yet implemented. Only path-level restrictions are active.
- The command filter uses regex patterns; sophisticated obfuscation may bypass it. Defense in depth is recommended.
- The daemon runs with the user's privileges. Do not expose the socket to untrusted processes.

## Best Practices

- Run arshy with the least-privileged user account
- Enable audit logging for production deployments
- Use `sandbox_paths` to restrict filesystem access
- Keep the blocked patterns list updated
- Review audit logs regularly for suspicious activity
