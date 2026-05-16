# Arshy Project Rules

## Shell Execution Policy (DOG FOODING)

**ALL shell commands MUST use arshy_exec. Bash tool is FORBIDDEN.**

- `cargo test` → `arshy_exec(action:"run", command:"cargo test")`
- `git log` → `arshy_exec(action:"run", command:"git log --oneline -5")`
- `ls`, `cat`, `grep`, `find` → `arshy_exec(action:"run", command:"...")`
- Directory navigation → `arshy_exec(action:"cd", command:"/path/to/dir")`

### Why
This project IS the shell. We eat our own dog food. Every command exercised through arshy is a real-world test of the product we're building. Using Bash to build a Bash replacement is self-defeating.

### Fallback
If `arshy_exec` returns `DaemonUnreachable`, run `scripts/restart.sh` to revive the daemon. Only if that fails, use Bash as last resort. The daemon has auto-restart (P0) — in normal operation this should never be needed.

### Commit Policy
- Run `cargo test` through arshy_exec before every commit
- Use arshy_exec for `git diff`, `git add`, `git commit`
- Never commit if arshy_exec is unavailable (fix the daemon first)
