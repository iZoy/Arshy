//! MCP stdio proxy — connects MCP clients (Claude Code, Cursor) to the arshy daemon.
//!
//! Architecture:
//! - Reads MCP JSON-RPC from stdin
//! - Forwards tool calls to daemon via `DaemonConnection`
//! - Receives daemon notifications and forwards as MCP notifications to stdout
//! - Auto-starts daemon if needed

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, DaemonConnection, Notification};
use arshy_lib::mcp::{instructions, protocol};
use arshy_lib::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

/// Run the MCP stdio proxy. Reads JSON-RPC from stdin, forwards to daemon, writes to stdout.
pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(proxy_main(config_path))
}

async fn proxy_main(config_path: Option<PathBuf>) -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides {
        config_path,
        ..Default::default()
    })?;

    let socket_path = cfg.daemon.expanded_socket_path();
    let stream = connect_or_start(&cfg, &socket_path).await?;
    let mut daemon = DaemonConnection::new(stream);

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = BufWriter::new(tokio::io::stdout());
    let mut line = String::new();

    loop {
        // Drain any pending notifications before blocking on stdin
        drain_and_forward_notifications(&mut daemon, &mut stdout).await?;

        line.clear();

        // Block on stdin. Between reads, notifications accumulate in the channel
        // and get drained at the top of the next loop iteration.
        let n = stdin.read_line(&mut line).await?;
        if n == 0 { break; }
        if line.trim().is_empty() { continue; }

        let request: serde_json::Value = serde_json::from_str(line.trim())
            .map_err(|e| arshy_lib::ArshyError::Mcp(e.to_string()))?;

        let method = request["method"].as_str().unwrap_or("");
        let id = request["id"].as_u64().unwrap_or(0);

        match method {
            "initialize" => handle_initialize(&mut stdout, id).await?,
            "tools/list" => handle_tools_list(&mut stdout, id).await?,
            "tools/call" => {
                // Try the tool call; on connection error, attempt reconnect
                match handle_tool_call(&mut daemon, &mut stdout, &request, id).await {
                    Ok(()) => {
                        drain_and_forward_notifications(&mut daemon, &mut stdout).await?;
                    }
                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                        tracing::warn!("daemon connection lost, attempting reconnect...");
                        match reconnect(&cfg, &socket_path).await {
                            Ok(new_conn) => {
                                daemon = new_conn;
                                tracing::info!("reconnected to daemon");
                                // Retry the tool call on the new connection
                                handle_tool_call(&mut daemon, &mut stdout, &request, id).await?;
                                drain_and_forward_notifications(&mut daemon, &mut stdout).await?;
                            }
                            Err(re) => {
                                tracing::error!("reconnect failed: {}", re);
                                write_json_error(&mut stdout, id, -32603,
                                    &format!("daemon connection lost: {}", e)).await?;
                            }
                        }
                    }
                    Err(e) => {
                        write_json_error(&mut stdout, id, -32603,
                            &format!("daemon error: {}", e)).await?;
                    }
                }
            }
            "notifications/initialized" | "notifications/cancelled" => {
                // No response for client notifications
            }
            _ => {
                write_json_error(&mut stdout, id, -32601,
                    &format!("unknown method: {}", method)).await?;
            }
        }
    }

    Ok(())
}

/// Drain all pending notifications from the daemon and write them to stdout.
async fn drain_and_forward_notifications(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
) -> Result<()> {
    for notif in daemon.drain_notifications() {
        write_mcp_notification(stdout, &notif).await?;
    }
    Ok(())
}

// ── MCP handlers ────────────────────────────────────────────────────────────

async fn handle_initialize(stdout: &mut BufWriter<tokio::io::Stdout>, id: u64) -> Result<()> {
    let mut notifications = HashMap::new();
    notifications.insert("diagnostic".to_string(), serde_json::Value::Object(Default::default()));

    let caps = protocol::ServerCapabilities {
        protocol_version: "2024-11-05".into(),
        server_info: protocol::ServerInfo {
            name: "arshy".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        capabilities: protocol::ServerFeatures {
            tools: HashMap::new(),
            notifications,
        },
        instructions: Some(instructions::default_instructions()),
    };
    write_json_response(stdout, id, &serde_json::to_value(&caps)?).await
}

async fn handle_tools_list(stdout: &mut BufWriter<tokio::io::Stdout>, id: u64) -> Result<()> {
    let tools = instructions::tool_definitions();
    write_json_response(stdout, id, &serde_json::json!({ "tools": tools })).await
}

async fn handle_tool_call(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    request: &serde_json::Value,
    id: u64,
) -> Result<()> {
    let tool_name = request["params"]["name"].as_str().unwrap_or("");
    let args = request["params"]["arguments"].clone();
    let ipc_method = mcp_tool_to_ipc_method(tool_name);

    let response = daemon.send_request(ipc_method, args).await?;
    let text = serde_json::to_string_pretty(&response.result).unwrap_or_default();

    let mcp_response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{"type": "text", "text": text}]
        }
    });

    let mut json = serde_json::to_vec(&mcp_response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}

// ── Daemon notification → MCP notification mapping ──────────────────────────

/// Forward a daemon IPC notification as an MCP `notifications/message`.
async fn write_mcp_notification(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    notif: &Notification,
) -> Result<()> {
    let (level, logger, data) = match notif.method.as_str() {
        "task/update" => {
            let task_id = notif.params["task_id"].as_str().unwrap_or("");
            let status = notif.params["status"].as_str().unwrap_or("");
            (protocol::LogLevel::Info, "arshy", serde_json::json!({
                "event": "task_update",
                "task_id": task_id,
                "status": status,
            }))
        }
        "task/complete" => {
            let task_id = notif.params["task_id"].as_str().unwrap_or("");
            let exit_code = notif.params["exit_code"].as_i64().unwrap_or(-1);
            let level = if exit_code == 0 {
                protocol::LogLevel::Info
            } else {
                protocol::LogLevel::Error
            };
            (level, "arshy", serde_json::json!({
                "event": "task_complete",
                "task_id": task_id,
                "exit_code": exit_code,
            }))
        }
        "diagnostic" => {
            let event = &notif.params["event"];
            let severity = event["severity"].as_str().unwrap_or("info");
            let level = match severity {
                "error" => protocol::LogLevel::Error,
                "warning" => protocol::LogLevel::Warning,
                "debug" => protocol::LogLevel::Debug,
                _ => protocol::LogLevel::Info,
            };
            (level, "arshy.parser", serde_json::json!({
                "event": "diagnostic",
                "task_id": notif.params["task_id"].as_str().unwrap_or(""),
                "type": event["type"].as_str().unwrap_or(""),
                "severity": severity,
                "message": event["message"].as_str().unwrap_or(""),
                "location": event.get("location"),
            }))
        }
        "daemon/shutdown" => {
            (protocol::LogLevel::Warning, "arshy.daemon", serde_json::json!({
                "event": "shutdown",
                "reason": notif.params["reason"].as_str().unwrap_or(""),
            }))
        }
        _ => return Ok(()), // Unknown notification, skip
    };

    let mcp_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": {
            "level": level,
            "logger": logger,
            "data": data,
        }
    });

    let mut json = serde_json::to_vec(&mcp_notif)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

async fn connect_or_start(cfg: &Config, socket_path: &std::path::Path) -> Result<tokio::net::UnixStream> {
    match ipc::connect(socket_path).await {
        Ok(s) => Ok(s),
        Err(_) if cfg.daemon.auto_start => {
            start_daemon()?;
            for _ in 0..25 {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if let Ok(s) = ipc::connect(socket_path).await {
                    return Ok(s);
                }
            }
            Err(arshy_lib::ArshyError::DaemonUnreachable(
                "daemon did not start within 5s".into(),
            ))
        }
        Err(e) => Err(e),
    }
}

fn start_daemon() -> Result<()> {
    std::process::Command::new("arshyd")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| arshy_lib::ArshyError::DaemonUnreachable(format!("spawn arshyd: {}", e)))?;
    Ok(())
}

/// Check if an error indicates a broken daemon connection.
fn is_connection_error(e: &arshy_lib::ArshyError) -> bool {
    let msg = format!("{}", e);
    msg.contains("connection closed")
        || msg.contains("timed out")
        || msg.contains("response channel dropped")
        || msg.contains("broken pipe")
        || msg.contains("Connection refused")
        || msg.contains("No such file")
}

/// Attempt to reconnect to the daemon.
async fn reconnect(
    cfg: &Config,
    socket_path: &std::path::Path,
) -> Result<DaemonConnection> {
    let stream = connect_or_start(cfg, socket_path).await?;
    Ok(DaemonConnection::new(stream))
}

fn mcp_tool_to_ipc_method(tool_name: &str) -> &str {
    match tool_name {
        "arshy_run" => ipc::METHOD_RUN,
        "arshy_query" => ipc::METHOD_QUERY,
        "arshy_list" => ipc::METHOD_LIST,
        "arshy_kill" => ipc::METHOD_KILL,
        "arshy_tail" => ipc::METHOD_TAIL,
        _ => ipc::METHOD_RUN,
    }
}

async fn write_json_response(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: u64,
    result: &serde_json::Value,
) -> Result<()> {
    let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let mut json = serde_json::to_vec(&response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}

async fn write_json_error(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: u64,
    code: i64,
    message: &str,
) -> Result<()> {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    });
    let mut json = serde_json::to_vec(&response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}
