//! Daemon connection management — spawn, retry, reconnect, and circuit breaker.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, DaemonConnection, Notification};
use arshy_lib::Result;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;

use super::protocol::{notification_to_json, write_mcp_notification};

pub(crate) async fn ensure_daemon_up(
    daemon: &mut DaemonConnection,
    notif_rx: &mut mpsc::Receiver<Notification>,
    cfg: &Config,
    socket_path: &std::path::Path,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    pending_notifs: &mut Vec<Notification>,
) -> Result<()> {
    // Drain remaining notifications from the stale channel before replacing it.
    while let Ok(n) = notif_rx.try_recv() {
        pending_notifs.push(n);
    }
    if !pending_notifs.is_empty() {
        flush_batch(stdout, pending_notifs).await?;
    }
    match connect_with_retry(cfg, socket_path, 3, &[500, 1000, 2000], false).await {
        Ok((new_conn, new_notif_rx)) => {
            *daemon = new_conn;
            *notif_rx = new_notif_rx;
            tracing::info!("spawned/recovered daemon on demand");
            Ok(())
        }
        Err(_) => {
            record_daemon_crash();
            tracing::error!("daemon unreachable after retries — run `arshy daemon start`");
            Err(arshy_lib::ArshyError::DaemonUnreachable(
                "arshyd daemon is not running or unreachable. Run `arshy daemon start` to start it.".into(),
            ))
        }
    }
}

/// Drain pending notifications from the channel (non-blocking) and forward
/// to stdout. Called after a tool call completes.
pub(crate) async fn drain_pending(
    notif_rx: &mut mpsc::Receiver<Notification>,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    pending: &mut Vec<Notification>,
    max_batch: usize,
) -> Result<()> {
    while let Ok(n) = notif_rx.try_recv() {
        pending.push(n);
        if pending.len() >= max_batch {
            flush_batch(stdout, pending).await?;
        }
    }
    if !pending.is_empty() {
        flush_batch(stdout, pending).await?;
    }
    Ok(())
}

/// Write all pending batched notifications as a single MCP notification message.
pub(crate) async fn flush_batch(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    batch: &mut Vec<Notification>,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }

    if batch.len() == 1 {
        write_mcp_notification(stdout, &batch[0]).await?;
    } else {
        // Coalesce: send as a single notifications/message with a JSON array payload
        let items: Vec<serde_json::Value> = batch.iter().filter_map(notification_to_json).collect();

        if !items.is_empty() {
            let mcp_notif = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/message",
                "params": {
                    "level": "info",
                    "logger": "arshy.batch",
                    "data": {
                        "event": "batch",
                        "count": items.len(),
                        "items": items,
                    }
                }
            });

            let mut json = serde_json::to_vec(&mcp_notif)?;
            json.push(b'\n');
            stdout.write_all(&json).await?;
            stdout.flush().await?;
        }
    }

    batch.clear();
    Ok(())
}

pub(crate) async fn connect_or_start(
    cfg: &Config,
    socket_path: &std::path::Path,
) -> Result<tokio::net::UnixStream> {
    match ipc::connect(socket_path).await {
        Ok(s) => Ok(s),
        Err(_) if cfg.daemon.auto_start => {
            // If the socket file exists but connection failed, the daemon process
            // may be dead (stale socket). Clean it up before spawning.
            if socket_path.exists() {
                tracing::warn!("stale socket detected, removing {}", socket_path.display());
                let _ = std::fs::remove_file(socket_path);
            }
            start_daemon()?;
            for _ in 0..40 {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if let Ok(s) = ipc::connect(socket_path).await {
                    return Ok(s);
                }
            }
            // Daemon failed to start — record crash for circuit breaker
            record_daemon_crash();
            Err(arshy_lib::ArshyError::DaemonUnreachable("daemon did not start within 10s".into()))
        }
        Err(e) => Err(e),
    }
}

/// Spawn the arshy daemon — the SINGLE spawn entry point for every caller
/// (MCP proxy auto-start, CLI `daemon start`, `install`, bash wrapper).
///
/// Guarantees, uniformly:
/// 1. **Sibling binary** — resolves `arshyd` next to the running `arshy`
///    (never a stale PATH-resolved binary from an old `cargo install`).
/// 2. **Session detachment** — `setsid()` so the daemon survives the caller's
///    session teardown (agent tool calls, CI steps, short-lived scripts).
/// 3. **Spawn-lock** — atomic `/tmp/arshyd.spawn-lock` prevents duplicate spawns.
/// 4. **Circuit breaker** — suppresses auto-start after 5 crashes in 2 minutes.
pub(crate) fn start_daemon() -> Result<()> {
    use libc;
    use std::fs::OpenOptions;
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    // ── Circuit breaker check ───────────────────────────────────────────
    if !check_circuit_breaker() {
        return Err(arshy_lib::ArshyError::DaemonUnreachable(
            "daemon crash-loop detected, auto-start suppressed".into(),
        ));
    }

    // Atomic lock — if another proxy already spawned (or is spawning) the daemon,
    // this will fail and we'll just wait for the socket to appear.
    let lock_path = std::path::PathBuf::from("/tmp/arshyd.spawn-lock");
    let mut lock_file = match OpenOptions::new().create_new(true).write(true).open(&lock_path) {
        Ok(f) => f,
        Err(_) => {
            tracing::debug!("spawn lock held by another process, skipping spawn");
            return Ok(());
        }
    };
    // Write our PID into the lock file for debugging
    let _ = writeln!(lock_file, "{}", std::process::id());

    // Resolve the arshyd binary path — prefer the sibling of the REAL current
    // exe, falling back to PATH. `canonicalize` matters: when arshy runs
    // through the shell shims (~/.arshy/bin/bash -> arshy), current_exe()
    // may return the symlink path (.arshy/bin/bash) whose directory has no
    // arshyd sibling — the daemon would never start for agent bash calls.
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(&p).ok().or(Some(p)))
        .and_then(|p| p.parent().map(|d| d.join("arshyd")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("arshyd"));

    let mut cmd = std::process::Command::new(&path);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Detach from the spawning process's process group / controlling terminal so the
    // daemon survives even if the parent (e.g. the agent's shell) exits before the
    // daemon's idle-timeout. This decouples daemon lifetime from the caller.
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(|e| {
        let _ = std::fs::remove_file(&lock_path);
        arshy_lib::ArshyError::DaemonUnreachable(format!("spawn {}: {}", path.display(), e))
    })?;

    // Release the lock once the daemon has started.
    drop(lock_file);
    let _ = std::fs::remove_file(&lock_path);

    tracing::info!("spawned arshyd (pid {}) from {}", child.id(), path.display());
    Ok(())
}

// ── Circuit breaker: prevents daemon crash-looping ───────────────────────

/// Shared crash timestamp log. If more than MAX_CRASHES occur within
/// CRASH_WINDOW, auto-start is suppressed.
static CRASH_LOG: Mutex<Option<Vec<Instant>>> = Mutex::new(None);
const MAX_CRASHES: usize = 5;
const CRASH_WINDOW_SECS: u64 = 120;

fn prune_crash_log(log: &mut Vec<Instant>) {
    let cutoff = Instant::now() - Duration::from_secs(CRASH_WINDOW_SECS);
    log.retain(|t| *t > cutoff);
}

fn check_circuit_breaker() -> bool {
    let mut guard = CRASH_LOG.lock().expect("CRASH_LOG poisoned");
    let log = guard.get_or_insert_with(Vec::new);
    prune_crash_log(log);
    if log.len() >= MAX_CRASHES {
        tracing::error!(
            "circuit breaker tripped: {} daemon crashes in {}s, refusing auto-start",
            log.len(),
            CRASH_WINDOW_SECS
        );
        return false;
    }
    true
}

fn record_daemon_crash() {
    let mut guard = CRASH_LOG.lock().expect("CRASH_LOG poisoned");
    let log = guard.get_or_insert_with(Vec::new);
    prune_crash_log(log);
    log.push(Instant::now());
    tracing::warn!(
        "daemon crash recorded ({}/{}) — {} in last {}s",
        log.len(),
        MAX_CRASHES,
        log.len(),
        CRASH_WINDOW_SECS
    );
}

/// Check if an error indicates a broken daemon connection.
pub(crate) fn is_connection_error(e: &arshy_lib::ArshyError) -> bool {
    let msg = format!("{}", e);
    msg.contains("connection closed")
        || msg.contains("timed out")
        || msg.contains("response channel dropped")
        || msg.contains("broken pipe")
        || msg.contains("Connection refused")
        || msg.contains("No such file")
}

/// Connect to daemon with retry and optional auto-cd validation.
///
/// Used for both startup and reconnection. When `validate_with_cd` is true,
/// sends a METHOD_CD to verify the connection is alive; retries if it fails.
pub(crate) async fn connect_with_retry(
    cfg: &Config,
    socket_path: &std::path::Path,
    attempts: usize,
    backoff_ms: &[u64],
    validate_with_cd: bool,
) -> Result<(DaemonConnection, mpsc::Receiver<Notification>)> {
    let mut last_err = None;
    for attempt in 0..attempts {
        if attempt > 0 {
            let delay = backoff_ms.get(attempt - 1).unwrap_or(backoff_ms.last().unwrap_or(&500));
            tokio::time::sleep(Duration::from_millis(*delay)).await;
        }
        match connect_or_start(cfg, socket_path).await {
            Ok(stream) => {
                let (mut d, nr) = DaemonConnection::new(stream);
                if validate_with_cd {
                    let cd_ok = if let Ok(cwd) = std::env::current_dir() {
                        let cwd_str = cwd.to_string_lossy().to_string();
                        let params = serde_json::json!({ "command": cwd_str });
                        d.send_request_with_timeout(ipc::METHOD_CD, params, Duration::from_secs(3))
                            .await
                            .is_ok()
                    } else {
                        true
                    };
                    if !cd_ok {
                        tracing::warn!("auto-cd failed (attempt {})", attempt + 1);
                        last_err =
                            Some(arshy_lib::ArshyError::DaemonUnreachable("auto-cd failed".into()));
                        continue;
                    }
                }
                return Ok((d, nr));
            }
            Err(e) => {
                tracing::warn!("connection attempt {}/{} failed: {}", attempt + 1, attempts, e);
                last_err = Some(e);
            }
        }
    }
    Err(last_err
        .unwrap_or_else(|| arshy_lib::ArshyError::DaemonUnreachable("connection failed".into())))
}
