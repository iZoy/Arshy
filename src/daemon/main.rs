//! Arshy daemon — background process managing shell execution.

mod bus;
mod context;
mod exec;
mod ipc_handler;
mod lifecycle;
mod parser;
mod security;
mod store;
mod telemetry;

use arshy_lib::config::{Config, expand_path};
use arshy_lib::Result;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides::default())?;

    arshy_lib::config::init_logging(&cfg.daemon.log_level, &cfg.daemon.log_format);

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

    // Clean up any stale spawn-lock left by a crashed proxy.
    // Without this, the proxy's atomic create_new would fail forever.
    let spawn_lock_path = std::path::PathBuf::from("/tmp/arshyd.spawn-lock");
    if spawn_lock_path.exists() {
        let _ = std::fs::remove_file(&spawn_lock_path);
        tracing::info!("cleaned up stale spawn-lock");
    }

    // ── Database ───────────────────────────────────────────────────────────
    let db_dir = cfg.store.expanded_db_path();
    if let Some(parent) = db_dir.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let store = Arc::new(store::Store::open(&db_dir, cfg.store.wal_mode)?);
    store.initialize_schema()?;

    // SQLite integrity check
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

    // WAL checkpoint
    if cfg.store.wal_mode {
        if let Err(e) = store.wal_checkpoint() {
            tracing::debug!("WAL checkpoint: {}", e);
        }
    }

    // ── Security validation ──────────────────────────────────────────────────
    if cfg.daemon.sandbox_mode != "none" {
        return Err(arshy_lib::ArshyError::Config(format!(
            "sandbox_mode '{}' is not implemented; only 'none' is supported. \
             See https://github.com/izoy/arshy#sandbox for roadmap.",
            cfg.daemon.sandbox_mode
        )));
    }

    // ── Executor ───────────────────────────────────────────────────────────
    let parser_engine = Arc::new(parser::Engine::new(&cfg.parser)?);
    let event_bus = bus::EventBus::new();

    let exec_config = exec::ExecutorConfig {
        max_task_duration_ms: cfg.daemon.max_task_duration_ms,
        max_output_bytes: cfg.daemon.max_output_bytes,
        kill_graceful_ms: cfg.daemon.kill_graceful_ms,
        kill_force_ms: cfg.daemon.kill_force_ms,
    };
    let mut executor = exec::Executor::new(
        store.clone(),
        parser_engine.clone(),
        event_bus.clone(),
    ).with_config(exec_config).with_security(&cfg.security);

    if let Some(ref audit_path) = cfg.security.audit_log {
        let expanded = expand_path(std::path::Path::new(audit_path));
        let audit = Arc::new(security::AuditLog::new(&expanded)?);
        tracing::info!("audit log: {}", expanded.display());
        executor = executor.with_audit_log(audit);
    }

    let executor = Arc::new(executor);

    // ── Parser hot-reload watcher ─────────────────────────────────────────
    let _watcher = parser_engine.start_watcher()?;

    // ── Shutdown channel ───────────────────────────────────────────────────
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // ── Accept loop ────────────────────────────────────────────────────────
    let _ = tokio::fs::remove_file(&socket_path).await;
    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!("listening on {}", socket_path.display());

    let mut conn_id: u64 = 0;
    // Limit concurrent connections to avoid resource exhaustion.
    // MCP proxy typically uses 1–2 connections; this headroom handles bursts.
    let conn_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(64));

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
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

    // Wait for running tasks to finish (with a grace period)
    let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let running = store.list_tasks(Some("running"), 10_000)
            .map(|t| t.len())
            .unwrap_or(0);
        if running == 0 {
            tracing::info!("all tasks completed, clean shutdown");
            break;
        }
        if tokio::time::Instant::now() > drain_deadline {
            tracing::warn!("{} tasks still running after grace period, forcing shutdown", running);
            break;
        }
        tracing::debug!("waiting for {} running tasks to complete...", running);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // ── Cleanup ────────────────────────────────────────────────────────────
    lifecycle::remove_pid();
    let _ = tokio::fs::remove_file(&socket_path).await;
    tracing::info!("arshyd stopped");
    Ok(())
}

/// Wait for the shutdown watch channel to become true.
async fn wait_shutdown(mut rx: watch::Receiver<bool>) {
    let _ = rx.changed().await;
}
