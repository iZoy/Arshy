//! Arshy daemon — background process managing shell execution.

use arshy_lib::config::{expand_path, Config};
use arshy_lib::daemon::{
    bus, exec, ipc_handler, lifecycle, parser, reference, security, store, telemetry,
};
use arshy_lib::Result;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<()> {
    // `arshyd --version` / `arshyd -V` — print the version and exit, matching
    // the CLI binary so `arshy --version && arshyd --version` verifies both
    // halves of an install. Without this, --version was treated as a startup
    // flag and the daemon tried to boot (or failed with "already running").
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("arshyd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let cfg = Config::load(arshy_lib::config::CliOverrides::default())?;

    arshy_lib::config::init_logging(&cfg.daemon.log_level, &cfg.daemon.log_format);

    // Panic hook — log the panic message with backtrace before the process exits
    std::panic::set_hook(Box::new(|info| {
        let location =
            info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let payload = info.payload();
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            *s
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.as_str()
        } else {
            "(non-string panic payload)"
        };
        tracing::error!(
            location = location,
            panic.message = msg,
            "daemon panicked — collecting diagnostics before exit"
        );
        // Write a clear, actionable message to stderr before the process exits
        // so that auto-starting proxies and launchd/systemd see a useful message.
        eprintln!("FATAL: arshyd daemon panicked at {}: {}", location, msg);
        eprintln!("The daemon will restart automatically (KeepAlive).");
        eprintln!("If this persists, check the log: ~/.local/share/arshy/daemon.log");
        // Flush logs before the process aborts
    }));

    tracing::info!("arshyd v{} starting", env!("CARGO_PKG_VERSION"));

    // ── Lifecycle: PID + stale socket ───────────────────────────────────────
    if let Ok(pid) = lifecycle::check_running() {
        if pid > 0 {
            tracing::error!("daemon already running (pid {})", pid);
            std::process::exit(1);
        }
    }
    lifecycle::write_pid()?;

    let socket_path = cfg.daemon.expanded_socket_path();
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    lifecycle::cleanup_stale_socket(&socket_path);

    // ── Store (JSONL) ───────────────────────────────────────────────────────────
    let store_dir = cfg.store.expanded_store_dir();
    if let Some(parent) = store_dir.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let store = Arc::new(store::Store::open(&store_dir)?);
    store.initialize_schema()?;

    // Start background flush task — persists dirty state on change (battery-safe)
    let _flush_handle = store.start_flush_task();

    // Store integrity check
    if cfg.store.integrity_check {
        match store.integrity_check() {
            Ok(result) if result == "ok" => tracing::debug!("integrity check passed"),
            Ok(result) => tracing::warn!("integrity check: {}", result),
            Err(e) => tracing::error!("integrity check failed: {}", e),
        }
    }

    // Auto-prune on startup
    if cfg.store.auto_prune {
        match store.prune_older_than(cfg.store.prune_older_than_days) {
            Ok((dt, de)) if dt > 0 => tracing::info!("auto-prune: {} tasks, {} events", dt, de),
            Err(e) => tracing::warn!("auto-prune failed: {}", e),
            _ => tracing::debug!("auto-prune: nothing to prune"),
        }
    }

    // Clean up stale /tmp/.arshy-cwd/ symlinks from previous sessions
    store::prune::cleanup_stale_symlinks();

    // Probe user's login shell PATH once (cached globally).
    // Enriches the minimal launchd PATH with tools from ~/.cargo/bin, homebrew, etc.
    let _ = exec::pty::user_shell_path();

    // `allowed_cwds` is intentionally a cwd boundary guard. It does not claim
    // to isolate file access after a child process has started.
    let security = cfg.security.clone();

    // Create custom parser directories if they don't exist
    for dir in &cfg.parser.dirs {
        let expanded = arshy_lib::config::expand_path(dir);
        if !expanded.exists() {
            let _ = std::fs::create_dir_all(&expanded);
        }
    }

    // ── Executor ───────────────────────────────────────────────────────────
    let parser_engine = Arc::new(parser::Engine::new(&cfg.parser)?);
    let event_bus = bus::EventBus::new();

    let exec_config = exec::ExecutorConfig {
        max_task_duration_ms: cfg.daemon.max_task_duration_ms,
        max_output_bytes: cfg.daemon.max_output_bytes,
        kill_graceful_ms: cfg.daemon.kill_graceful_ms,
        kill_force_ms: cfg.daemon.kill_force_ms,
        max_concurrent_tasks: cfg.daemon.max_concurrent_tasks as usize,
    };
    let mut executor = exec::Executor::new(store.clone(), parser_engine.clone(), event_bus.clone())
        .with_config(exec_config)
        .with_security(&security)?
        .with_reference(Arc::new(reference::ReferenceTable::load()?));

    if let Some(ref audit_path) = cfg.security.audit_log {
        let expanded = expand_path(std::path::Path::new(audit_path));
        let audit = Arc::new(security::AuditLog::new(&expanded)?);
        tracing::info!("audit log: {}", expanded.display());
        executor = executor.with_audit_log(audit);
    }

    let executor = Arc::new(executor);

    // ── Parser hot-reload watcher ─────────────────────────────────────────
    let _watcher = parser_engine.start_watcher()?;

    // ── Reference-table hot-reload watcher ───────────────────────────────
    // User tables live in ~/.arshy/reference/*.toml; reload on change. The
    // directory may not exist yet — the watcher starts once it does.
    let _ref_watcher =
        parser::ParserWatcher::start(&[std::path::PathBuf::from("${HOME}/.arshy/reference")], {
            let executor = executor.clone();
            move || {
                if let Err(e) = executor.reload_reference() {
                    tracing::error!("reference hot-reload failed: {}", e);
                } else {
                    tracing::info!("reference tables hot-reloaded");
                }
            }
        })?;

    // ── Shutdown channel ───────────────────────────────────────────────────
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // ── Accept loop ────────────────────────────────────────────────────────
    let _ = tokio::fs::remove_file(&socket_path).await;
    let listener = UnixListener::bind(&socket_path)?;
    // Restrict socket to owner-only — prevents unauthorized local access
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    }
    // Signal a proxy waiting in `start_daemon` that the listener is ready.
    // The lock is scoped to this socket and is intentionally removed only
    // after a successful bind.
    let spawn_lock = socket_path.with_file_name(format!(
        "{}.spawn-lock",
        socket_path.file_name().and_then(|name| name.to_str()).unwrap_or("arshyd")
    ));
    let _ = std::fs::remove_file(&spawn_lock);
    tracing::info!("listening on {}", socket_path.display());

    let mut conn_id: u64 = 0;
    // Limit concurrent connections to avoid resource exhaustion.
    // MCP proxy typically uses 1–2 connections; this headroom handles bursts.
    let conn_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(64));

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                // UID security check – reject connections from processes with a different UID
                if !peer_uid_allowed(&stream) {
                    tracing::warn!("rejecting connection {} from different UID", conn_id);
                    continue;
                }
                let id = conn_id;
                conn_id = conn_id.wrapping_add(1);
                telemetry::record_connection_accepted();

                let exec = executor.clone();
                let db = store.clone();
                let bus = event_bus.clone();
                let sd = shutdown_tx.clone();
                let permit = conn_semaphore.clone();

                tokio::spawn(async move {
                    let _permit = permit.acquire().await;
                    if let Err(e) = _permit {
                        tracing::error!("conn {} semaphore closed: {}", id, e);
                        return;
                    }
                    tracing::debug!("conn {} established", id);
                    if let Err(e) = ipc_handler::handle(stream, id, exec, db, bus, sd).await {
                        tracing::error!("conn {} error: {}", id, e);
                    }
                    tracing::debug!("conn {} closed", id);
                });
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received ctrl-c, shutting down");
                break;
            }
            _ = wait_shutdown(shutdown_rx.clone()) => {
                tracing::info!("received shutdown request");
                break;
            }
            // Event-driven idle exit: one deadline is reset by task activity.
            // No fixed-interval polling runs while the daemon is idle.
            _ = store.wait_until_idle(cfg.daemon.idle_timeout_secs) => {
                tracing::info!(
                    "idle for {}s, shutting down",
                    cfg.daemon.idle_timeout_secs
                );
                break;
            }
        }
    }

    // ── Graceful shutdown: stop accepting, drain running tasks ──────────────
    tracing::info!("shutting down, draining running tasks...");

    // Notify all connections that daemon is shutting down
    let sd_event = bus::BusEvent {
        connection_id: 0,
        kind: bus::BusEventKind::DaemonShutdown {
            reason: "shutdown".into(),
            grace_period_ms: 30_000,
        },
    };
    event_bus.publish(sd_event);

    // Remove socket to reject new connections while tasks drain
    let _ = tokio::fs::remove_file(&socket_path).await;

    // Phase 1: wait 5s for natural completion
    let phase1 = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let running = store
            .list_tasks(Some("running"), 10_000)
            .map(|t: Vec<arshy_lib::ipc::Task>| t.len())
            .unwrap_or(0);
        if running == 0 {
            tracing::info!("all tasks completed, clean shutdown");
            break;
        }
        if tokio::time::Instant::now() >= hard_deadline {
            tracing::warn!("{} tasks still running after 30s grace, forcing exit", running);
            break;
        }
        // Phase 2: after 5s, actively kill remaining tasks with process-tree signals
        if tokio::time::Instant::now() >= phase1 {
            executor.kill_all().await;
        }
        tracing::debug!("waiting for {} running tasks to complete...", running);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // ── Cleanup ────────────────────────────────────────────────────────────
    // Flush any pending store changes before exit
    if let Err(e) = store.flush() {
        tracing::warn!("final store flush failed: {}", e);
    }
    lifecycle::remove_pid();
    let _ = tokio::fs::remove_file(&socket_path).await;
    tracing::info!("arshyd stopped");
    Ok(())
}

/// Wait for the shutdown watch channel to become true.
async fn wait_shutdown(mut rx: watch::Receiver<bool>) {
    let _ = rx.changed().await;
}

/// Returns `true` if the peer of `stream` runs under the same effective UID as
/// this process. Connections from other local users are rejected — defense in
/// depth so that even a 0600 socket can't be abused by a different local user
/// (e.g. a spawned child or another session).
#[cfg(unix)]
fn peer_uid_allowed(stream: &tokio::net::UnixStream) -> bool {
    match stream.peer_cred() {
        Ok(cred) => cred.uid() == unsafe { libc::getuid() },
        Err(e) => {
            tracing::error!("failed to get peer credentials: {}", e);
            false
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::net::UnixListener;

    /// A socket pair created within the same process shares the current UID, so
    /// the accept-side stream must be allowed. This drives the real
    /// `peer_cred()` + UID comparison path end to end (no mocking).
    #[tokio::test]
    async fn peer_uid_allowed_accepts_same_uid_connection() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("peer-test.sock");
        let listener = UnixListener::bind(&path).unwrap();
        // Connect from the same process (same effective UID).
        let _client = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        assert!(peer_uid_allowed(&server));
    }

    /// The function must be total and never panic, even when called repeatedly
    /// on a valid socket. (The reject-on-different-UID branch cannot be
    /// exercised without forging another UID, which requires privilege changes
    /// unavailable in CI — hence it is covered here only for totality.)
    #[tokio::test]
    async fn peer_uid_allowed_is_total_and_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("peer-test2.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let _client = tokio::net::UnixStream::connect(&path).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        assert!(peer_uid_allowed(&server));
        assert!(peer_uid_allowed(&server));
    }
}
