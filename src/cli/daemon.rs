//! Daemon management and generic local diagnostics.

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, Request, METHOD_SHUTDOWN, METHOD_STATS, METHOD_STATUS};
use arshy_lib::Result;
use std::path::PathBuf;

use super::tasks::connect;
use crate::DaemonAction;

pub(crate) async fn daemon_status(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_STATUS.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

pub(crate) async fn daemon_stats(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    format: &str,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_STATS.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&response.result)?),
        _ => eprint!("{}", super::render::render_stats(&response.result)),
    }
    Ok(())
}

pub(crate) async fn daemon_action(
    action: DaemonAction,
    config_path: Option<PathBuf>,
    log_level: Option<String>,
) -> Result<()> {
    match action {
        DaemonAction::Start => daemon_start(config_path).await,
        DaemonAction::Stop => daemon_stop(config_path, log_level).await,
        DaemonAction::Restart => {
            let _ = daemon_stop(config_path.clone(), log_level.clone()).await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            daemon_start(config_path).await
        }
    }
}

async fn daemon_start(config_path: Option<PathBuf>) -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides { config_path, ..Default::default() })
        .unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();

    if ipc::connect(&socket_path).await.is_ok() {
        println!("daemon is already running");
        return Ok(());
    }

    crate::proxy::start_daemon(&socket_path)?;
    for _ in 0..25 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if ipc::connect(&socket_path).await.is_ok() {
            println!("daemon started");
            return Ok(());
        }
    }
    Err(arshy_lib::ArshyError::DaemonUnreachable("daemon did not start within 5s".into()))
}

async fn daemon_stop(config_path: Option<PathBuf>, log_level: Option<String>) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_SHUTDOWN.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

fn which_arshy_path() -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let path = PathBuf::from(dir).join("arshy");
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn which_arshyd() -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let path = PathBuf::from(dir).join("arshyd");
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Check the generic installation without inspecting or modifying Agent files.
pub(crate) fn doctor(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    _legacy_agent: Option<&str>,
) -> Result<()> {
    let mut passed = 0u32;
    let mut failed = 0u32;
    let check = |label: &str, ok: bool, hint: &str, passed: &mut u32, failed: &mut u32| {
        if ok {
            println!("  ✓ {label}");
            *passed += 1;
        } else {
            println!("  ✗ {label} — {hint}");
            *failed += 1;
        }
    };

    println!("arshy doctor — generic MCP diagnostics\n");
    check(
        "arshy in PATH",
        which_arshy_path().is_some(),
        "install the release binary",
        &mut passed,
        &mut failed,
    );
    check(
        "arshyd in PATH",
        which_arshyd().is_some(),
        "install the matching release",
        &mut passed,
        &mut failed,
    );

    let cfg = Config::load(arshy_lib::config::CliOverrides {
        config_path,
        log_level,
        ..Default::default()
    })
    .unwrap_or_default();
    check(
        "daemon socket exists",
        cfg.daemon.expanded_socket_path().exists(),
        "run `arshy daemon start`",
        &mut passed,
        &mut failed,
    );
    check(
        "MCP stdio command is resolvable",
        std::env::current_exe().is_ok(),
        "run `arshy mcp config`",
        &mut passed,
        &mut failed,
    );

    println!("\n{} passed, {} failed", passed, failed);
    if failed == 0 {
        println!("MCP entrypoint: arshy mcp serve");
        Ok(())
    } else {
        Err(arshy_lib::ArshyError::Other(format!("doctor found {failed} problem(s)")))
    }
}
