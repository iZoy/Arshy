# Known issues in v0.1.0-alpha.1

- The MCP result contract may change before v0.1.0.
- Only macOS and Linux are supported. WSL and native Windows are not validated.
- Arshy is not a filesystem sandbox; allowed_cwds checks only the starting cwd.
- Commands that exceed capture limits are drained but their retained output is
  truncated. A single line is capped before UTF-8 decoding.
- Parser coverage varies by tool and version. Captured text is available through
  arshy_task with action raw for persisted structured tasks, subject to the
  output limits and UTF-8/line rendering described in the IPC reference.
- Efficiency depends on the command, client, model, and follow-up behavior. No
  fixed token-saving percentage is claimed.

Report a reproducible problem through the matching GitHub Issue form.
