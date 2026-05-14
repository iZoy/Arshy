//! Arshy daemon — background process managing shell execution.

mod bus;
mod context;
mod exec;
mod ipc_handler;
mod parser;
mod security;
mod store;

use arshy_lib::config::{Config, expand_path};
use arshy_lib::Result;
use std::sync::Arc;
use tokio::net::UnixListener;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides::default())?;

    arshy_lib::config::init_logging(&cfg.daemon.log_level, &cfg.daemon.log_format);

    tracing::info!("arshyd v{} starting", env!("CARGO_PKG_VERSION"));
    tracing::debug!("config loaded");

    let db_dir = cfg.store.expanded_db_path();
    if let Some(parent) = db_dir.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let store = Arc::new(store::Store::open(&db_dir, cfg.store.wal_mode)?);
    store.initialize_schema()?;

    // SQLite integrity check (configurable)
    if cfg.store.integrity_check {
        match store.integrity_check() {
            Ok(result) => {
                if result == "ok" {
                    tracing::debug!("SQLite integrity check passed");
                } else {
                    tracing::warn!("SQLite integrity check: {}", result);
                }
            }
            Err(e) => {
                tracing::error!("SQLite integrity check failed: {}", e);
            }
        }
    }

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
        parser_engine,
        event_bus.clone(),
    ).with_config(exec_config).with_security(&cfg.security);

    // Audit log (if configured)
    if let Some(ref audit_path) = cfg.security.audit_log {
        let expanded = expand_path(std::path::Path::new(audit_path));
        let audit = Arc::new(security::AuditLog::new(&expanded)?);
        tracing::info!("audit log: {}", expanded.display());
        executor = executor.with_audit_log(audit);
    }

    let executor = Arc::new(executor);

    let socket_path = cfg.daemon.expanded_socket_path();
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

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

                tokio::spawn(async move {
                    tracing::debug!("conn {} established", id);
                    if let Err(e) = ipc_handler::handle(stream, id, exec, db, bus).await {
                        tracing::error!("conn {} error: {}", id, e);
                    }
                    tracing::debug!("conn {} closed", id);
                });
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                break;
            }
        }
    }

    let _ = tokio::fs::remove_file(&socket_path).await;
    tracing::info!("arshyd stopped");
    Ok(())
}
