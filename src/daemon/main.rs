//! Arshy daemon — background process managing shell execution.

mod bus;
mod context;
mod exec;
mod ipc_handler;
mod lifecycle;
mod parser;
mod security;
mod store;

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

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                let id = conn_id;
                conn_id += 1;

                let exec = executor.clone();
                let db = store.clone();
                let bus = event_bus.clone();
                let sd = shutdown_tx.clone();

                tokio::spawn(async move {
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

    // ── Cleanup ────────────────────────────────────────────────────────────
    let _ = tokio::fs::remove_file(&socket_path).await;
    lifecycle::remove_pid();
    tracing::info!("arshyd stopped");
    Ok(())
}

/// Wait for the shutdown watch channel to become true.
async fn wait_shutdown(mut rx: watch::Receiver<bool>) {
    let _ = rx.changed().await;
}
