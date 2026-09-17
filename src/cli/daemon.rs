//! Daemon management and generic local diagnostics.

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, Request, METHOD_SHUTDOWN, METHOD_STATS, METHOD_STATUS};
use arshy_lib::Result;
use serde::Serialize;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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

    if daemon_ready(&socket_path).await {
        println!("daemon is already running");
        return Ok(());
    }

    crate::proxy::start_daemon(&socket_path)?;
    for _ in 0..25 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if daemon_ready(&socket_path).await {
            println!("daemon started");
            return Ok(());
        }
    }
    Err(arshy_lib::ArshyError::DaemonUnreachable("daemon did not start within 5s".into()))
}

/// A socket can exist while the daemon is still loading parsers and opening
/// its store. Probe the status method so start/restart only returns after the
/// IPC handler is ready for real requests.
async fn daemon_ready(socket_path: &std::path::Path) -> bool {
    let Ok(mut connection) = ipc::connect(socket_path).await else {
        return false;
    };
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_STATUS.into(),
        params: serde_json::json!({}),
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        ipc::send_request(&mut connection, &request),
    )
    .await
    .is_ok_and(|result| result.is_ok())
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

#[derive(Serialize)]
struct DoctorCheck {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'static str>,
}

#[derive(Serialize)]
struct ProtocolCheck {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    negotiated_version: Option<String>,
}

#[derive(Serialize)]
struct DoctorReport {
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    daemon: DoctorCheck,
    protocol: ProtocolCheck,
    parser_count: Option<u64>,
}

fn doctor_error_message(code: &'static str) -> &'static str {
    match code {
        "executable_unavailable" => "cannot locate the current executable",
        "protocol_process_failed" => "MCP server could not be started",
        "protocol_write_failed" => "MCP initialize request could not be written",
        "protocol_timeout" => "MCP initialize response timed out",
        "protocol_handshake_failed" => "MCP initialize response was invalid",
        "daemon_version_mismatch" => "daemon version does not match the client",
        "daemon_status_failed" => "daemon status request failed",
        "daemon_unreachable" => "daemon is not reachable",
        _ => "doctor check failed",
    }
}

impl DoctorCheck {
    fn ok() -> Self {
        Self { status: "ok", code: None, message: None }
    }

    fn failed(code: &'static str) -> Self {
        Self { status: "failed", code: Some(code), message: Some(doctor_error_message(code)) }
    }
}

impl ProtocolCheck {
    fn ok(version: String) -> Self {
        Self { status: "ok", code: None, message: None, negotiated_version: Some(version) }
    }

    fn failed(code: &'static str) -> Self {
        Self {
            status: "failed",
            code: Some(code),
            message: Some(doctor_error_message(code)),
            negotiated_version: None,
        }
    }

    fn skipped(code: &'static str) -> Self {
        Self {
            status: "skipped",
            code: Some(code),
            message: Some(doctor_error_message(code)),
            negotiated_version: None,
        }
    }
}

async fn check_mcp_protocol(config_path: Option<&std::path::Path>) -> ProtocolCheck {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => {
            return ProtocolCheck::failed("executable_unavailable");
        }
    };
    let mut command = tokio::process::Command::new(executable);
    if let Some(path) = config_path {
        command.arg("--config").arg(path);
    }
    command
        .args(["mcp", "serve"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return ProtocolCheck::failed("protocol_process_failed");
        }
    };
    let request = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"arshy-doctor\",\"version\":\"1\"}}}\n";
    let Some(mut stdin) = child.stdin.take() else {
        return ProtocolCheck::failed("protocol_process_failed");
    };
    if stdin.write_all(request).await.is_err() || stdin.flush().await.is_err() {
        let _ = child.kill().await;
        return ProtocolCheck::failed("protocol_write_failed");
    }
    drop(stdin);
    let Some(stdout) = child.stdout.take() else {
        return ProtocolCheck::failed("protocol_process_failed");
    };
    let mut line = String::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        BufReader::new(stdout).read_line(&mut line),
    )
    .await;
    let _ = child.kill().await;
    let value = match read {
        Ok(Ok(read)) if read > 0 => serde_json::from_str::<serde_json::Value>(&line).ok(),
        Ok(_) => None,
        Err(_) => {
            return ProtocolCheck::failed("protocol_timeout");
        }
    };
    let negotiated = value
        .as_ref()
        .and_then(|response| response["result"]["protocolVersion"].as_str())
        .map(String::from);
    match negotiated {
        Some(version) => ProtocolCheck::ok(version),
        None => ProtocolCheck::failed("protocol_handshake_failed"),
    }
}

/// Check the generic installation without inspecting or modifying Agent files.
pub(crate) async fn doctor(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    _legacy_agent: Option<&str>,
    format: &str,
) -> Result<()> {
    let mut parser_count = None;
    // Use the same connection path as normal CLI operations so a fresh
    // installation can diagnose and auto-start its daemon in one command.
    let daemon = match connect(config_path.clone(), log_level).await {
        Ok(mut connection) => {
            let request = Request {
                jsonrpc: "2.0".into(),
                id: 1,
                method: METHOD_STATUS.into(),
                params: serde_json::json!({}),
            };
            match ipc::send_request(&mut connection, &request).await {
                Ok(response)
                    if response.result["version"].as_str() == Some(env!("CARGO_PKG_VERSION")) =>
                {
                    parser_count = response.result["parser_count"].as_u64();
                    DoctorCheck::ok()
                }
                Ok(_) => DoctorCheck::failed("daemon_version_mismatch"),
                Err(_) => DoctorCheck::failed("daemon_status_failed"),
            }
        }
        Err(_) => DoctorCheck::failed("daemon_unreachable"),
    };
    let protocol = if daemon.status == "ok" {
        check_mcp_protocol(config_path.as_deref()).await
    } else {
        ProtocolCheck::skipped("daemon_unreachable")
    };
    let failed = daemon.status != "ok" || protocol.status != "ok";
    let report = DoctorReport {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        daemon,
        protocol,
        parser_count,
    };

    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        "text" => {
            println!("arshy doctor — generic MCP diagnostics\n");
            println!("  version: {}", report.version);
            println!("  platform: {}-{}", report.os, report.arch);
            print_doctor_check(
                "daemon",
                report.daemon.status,
                report.daemon.code,
                report.daemon.message,
            );
            print_doctor_check(
                "protocol",
                report.protocol.status,
                report.protocol.code,
                report.protocol.message,
            );
            println!(
                "  parsers: {}",
                report.parser_count.map_or_else(|| "unavailable".into(), |v| v.to_string())
            );
        }
        other => {
            return Err(arshy_lib::ArshyError::Other(format!(
                "unknown doctor format '{other}'; expected text or json"
            )));
        }
    }
    if failed {
        Err(arshy_lib::ArshyError::Other("doctor checks failed".into()))
    } else {
        Ok(())
    }
}

fn print_doctor_check(label: &str, status: &str, code: Option<&str>, message: Option<&str>) {
    match (code, message) {
        (Some(code), Some(message)) => println!("  {label}: {status} ({code}) — {message}"),
        _ => println!("  {label}: {status}"),
    }
}
