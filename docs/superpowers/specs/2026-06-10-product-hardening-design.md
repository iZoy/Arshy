# Product Hardening Design

**Date:** 2026-06-10
**Status:** Draft
**Scope:** 4 P0 hardening items from quality audit

## Background

Quality audit rated arshy B+ with 4 blocking production issues:
1. No rate limiting — agent loops can DOS the machine
2. Only 7 security rules — easily bypassed
3. No distribution channel — users must build from source
4. 3 ignored integration tests — no binary-level smoke testing

## Subsystem 1: Token Bucket Rate Limiter

### Problem

An AI agent in a loop can fire unlimited commands per second, exhausting system resources.

### Design

**New file:** `src/daemon/security/ratelimit.rs`

```rust
pub struct RateLimiter {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,        // tokens per second
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(max_tokens: f64, refill_rate: f64) -> Self;
    pub fn try_acquire(&mut self) -> bool;   // consume 1 token, refill first
}
```

**Algorithm:** Classic token bucket.
- Bucket starts full at `max_tokens` (burst capacity)
- Tokens refill at `refill_rate` per second (sustained rate)
- Each command consumes 1 token
- If no tokens available, command is rejected with error

**Config (in `arshy.toml`):**
```toml
[security.rate_limit]
enabled = true
max_commands_per_second = 10
burst = 20
```

**Integration:** In `Executor::run()`, before security filter check:
```rust
if self.rate_limiter.enabled() && !self.rate_limiter.try_acquire() {
    return Err(ArshyError::RateLimited);
}
```

**Error:** New JSON-RPC error code `RATE_LIMITED = -32005` with message "Rate limit exceeded. Max {N} commands/second."

**Testing:**
- Unit: acquire within burst, exhaust burst, refill after wait
- Unit: disabled limiter always allows
- Integration: rapid-fire 30 commands, verify some are rejected

### Files

| File | Action |
|------|--------|
| `src/daemon/security/ratelimit.rs` | Create |
| `src/daemon/security/mod.rs` | Modify — re-export |
| `src/daemon/exec/mod.rs` | Modify — integrate rate check |
| `src/ipc/mod.rs` | Modify — add RATE_LIMITED error code |
| `src/config/` | Modify — add rate_limit config |

## Subsystem 2: Security Rules TOML Configuration

### Problem

Only 7 hardcoded blocked patterns. No sudo blocking, no env exfiltration, no base64 bypass, no key theft.

### Design

**New file:** `parsers/security/default-rules.toml`

20+ rules organized by category:

| Category | Rules | Severity |
|----------|-------|----------|
| Filesystem destruction | `rm -rf /`, `rm -rf ~`, `mkfs`, `dd if=` | critical |
| Shell injection | `curl\|sh`, `wget\|sh`, `eval`, base64 decode to shell | critical |
| Privilege escalation | `sudo` (any command), `su -` | high |
| Credential exfiltration | `cat ~/.ssh/`, `env`, `printenv`, `/proc/self/environ` | high |
| Network abuse | `nc -l` (listener), `nmap`, `ssh-keygen -f` | medium |
| Container escape | `nsenter`, `chroot`, mount host paths | high |
| Fork bombs | `:(){ :|:& };:`, `.%00` | critical |

**Rule format:**
```toml
[[block]]
name = "rm-root"
pattern = 'rm\s+(-[a-zA-Z]*[rRfF]){2,}\s+/'
reason = "Recursive force delete of root filesystem"
severity = "critical"
```

**Loading:** Two-tier:
1. Built-in: `include_str!("../../../parsers/security/default-rules.toml")` compiled into binary
2. User override: `~/.config/arshy/security.toml` (if exists, merged/additive)

**Integration:** `CommandFilter` loads TOML rules at construction, compiles regex patterns. Same interface as current filter.

**Testing:**
- Unit: each rule matches its intended pattern
- Unit: each rule does NOT match safe commands (false positive test)
- Unit: user rules merge with built-in rules
- Integration: blocked commands return correct error

### Files

| File | Action |
|------|--------|
| `parsers/security/default-rules.toml` | Create |
| `src/daemon/security/filter.rs` | Modify — load from TOML |
| `src/config/` | Modify — user security.toml path |

## Subsystem 3: Distribution Channels

### Problem

No way to install except building from source.

### Design

**Cargo install:**
- Verify `cargo install --path .` works (binary definitions in Cargo.toml)
- Add `description`, `license`, `repository`, `keywords` to Cargo.toml

**Homebrew formula:**
- Create `Formula/arshy.rb` for `brew install --build-from-source`
- Formula uses `cargo install` internally

**GitHub Release CI:**
- `.github/workflows/release.yml`
- Trigger: push tag `v*`
- Matrix: macOS arm64, macOS x86_64, Linux x86_64
- Steps: build release binary → create tarball → upload to GitHub Release
- Auto-update Homebrew formula with new version + SHA256

### Files

| File | Action |
|------|--------|
| `Cargo.toml` | Modify — add metadata |
| `Formula/arshy.rb` | Create |
| `.github/workflows/release.yml` | Create |

## Subsystem 4: Integration Test Fix

### Problem

3 critical E2E tests are `#[ignore]` due to macOS sandbox blocking socket creation.

### Design

**Approach:** Fix tests to work in CI (Linux) and conditionally skip on macOS sandbox.

1. Remove `#[ignore]` from the 3 tests
2. Use `tempfile::TempDir` for socket path (not hardcoded `/tmp/arshy.sock`)
3. Add `serial_test` crate for `#[serial]` attribute to prevent parallel conflicts
4. Add conditional skip: `#[cfg_attr(target_os = "macos", ignore)]` if sandbox issues persist
5. Each test: spawn daemon subprocess → wait for socket (poll up to 5s) → connect → exercise → kill daemon → cleanup

**Tests to fix:**
- `daemon_starts_and_responds_to_health` — spawn daemon, verify health RPC
- `run_echo_and_get_result` — spawn daemon, run `echo hello`, verify structured result
- `run_command_with_parser` — spawn daemon, run failing command, verify parser output

### Files

| File | Action |
|------|--------|
| `tests/integration.rs` | Modify — fix 3 tests |
| `Cargo.toml` | Modify — add serial_test dev-dependency |

## Implementation Phases

### Phase 1: Security (highest impact)
1. Security rules TOML (Subsystem 2)
2. Rate limiter (Subsystem 1)

### Phase 2: Quality
3. Integration test fix (Subsystem 4)

### Phase 3: Distribution
4. Distribution channels (Subsystem 3)
