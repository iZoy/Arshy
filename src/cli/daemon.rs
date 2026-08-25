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

fn is_development_build(path: &std::path::Path) -> bool {
    let value = path.to_string_lossy();
    value.contains("/target/debug/") || value.contains("/target/release/")
}

fn sibling_binary(executable: Option<&std::path::Path>, name: &str) -> Option<PathBuf> {
    executable
        .and_then(|path| path.parent())
        .map(|parent| parent.join(name))
        .filter(|path| path.is_file())
}

/// Check the generic installation without inspecting or modifying Agent files.
pub(crate) fn doctor(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    _legacy_agent: Option<&str>,
) -> Result<()> {
    let mut passed = 0u32;
    let mut warnings = 0u32;
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

    let current_exe = std::env::current_exe().ok();
    let development_build = current_exe.as_deref().is_some_and(is_development_build);
    let arshy_in_path = which_arshy_path();
    let arshyd_in_path = which_arshyd();
    let arshyd_sibling = sibling_binary(current_exe.as_deref(), "arshyd");

    let check_binary = |label: &str,
                        in_path: bool,
                        sibling: bool,
                        hint: &str,
                        passed: &mut u32,
                        warnings: &mut u32,
                        failed: &mut u32| {
        if in_path {
            println!("  ✓ {label}");
            *passed += 1;
        } else if development_build && sibling {
            println!("  ! {label} — development build is not installed in PATH");
            *warnings += 1;
        } else {
            println!("  ✗ {label} — {hint}");
            *failed += 1;
        }
    };

    println!("arshy doctor — generic MCP diagnostics\n");
    check_binary(
        "arshy in PATH",
        arshy_in_path.is_some(),
        current_exe.is_some(),
        "install the release binary",
        &mut passed,
        &mut warnings,
        &mut failed,
    );
    check_binary(
        "arshyd in PATH",
        arshyd_in_path.is_some(),
        arshyd_sibling.is_some(),
        "install the matching release",
        &mut passed,
        &mut warnings,
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

    println!("\n{} passed, {} warnings, {} failed", passed, warnings, failed);
    if failed == 0 {
        println!("MCP entrypoint: arshy mcp serve");
        Ok(())
    } else {
        Err(arshy_lib::ArshyError::Other(format!("doctor found {failed} problem(s)")))
    }
}

#[cfg(test)]
mod tests {
    use super::{is_development_build, sibling_binary};
    use std::path::Path;

    #[test]
    fn detects_debug_and_release_build_paths() {
        assert!(is_development_build(Path::new("/repo/target/debug/arshy")));
        assert!(is_development_build(Path::new("/repo/target/release/arshy")));
        assert!(!is_development_build(Path::new("/Users/me/.local/bin/arshy")));
    }

    #[test]
    fn missing_sibling_is_not_considered_resolved() {
        assert!(sibling_binary(Some(Path::new("/definitely/missing/arshy")), "arshyd").is_none());
    }
}
