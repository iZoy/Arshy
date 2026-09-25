//! MCP protocol helpers — version negotiation, notification mapping, and JSON-RPC writes.

use arshy_lib::config::Config;
use arshy_lib::ipc::Notification;
use arshy_lib::mcp::protocol;
use arshy_lib::Result;
use tokio::io::{AsyncWriteExt, BufWriter};

/// Protocol versions this server implements (MCP spec releases).
///
/// We support the full range of stable spec releases. `handle_initialize`
/// echoes the client's requested version when it is one of these (newer
/// clients such as Claude Code 2.1+ negotiate with `2025-11-25`; answering
/// with a stale hardcoded version has caused real-world handshake failures);
/// unknown or missing versions fall back to the earliest stable release.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// Pick the protocol version to answer with: echo a supported client
/// request, otherwise fall back to the default.
pub(crate) fn negotiate_protocol_version(request: &serde_json::Value) -> String {
    match request["params"]["protocolVersion"].as_str() {
        Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v.to_string(),
        _ => DEFAULT_PROTOCOL_VERSION.to_string(),
    }
}

// ── Daemon notification → MCP notification mapping ──────────────────────────

/// Forward a daemon IPC notification as an MCP `notifications/message`.
pub(crate) async fn write_mcp_notification<W: tokio::io::AsyncWriteExt + Unpin>(
    stdout: &mut BufWriter<W>,
    notif: &Notification,
) -> Result<()> {
    let (level, logger, data) = match notif.method.as_str() {
        "task/update" => {
            let task_id = notif.params["task_id"].as_str().unwrap_or("");
            let status = notif.params["status"].as_str().unwrap_or("");
            (
                protocol::LogLevel::Info,
                "arshy",
                serde_json::json!({
                    "event": "task_update",
                    "task_id": task_id,
                    "status": status,
                }),
            )
        }
        "task/complete" => {
            let task_id = notif.params["task_id"].as_str().unwrap_or("");
            let status = notif.params["status"].as_str().unwrap_or("failed");
            let exit_code = notif.params["exit_code"].as_i64().unwrap_or(-1);
            let level =
                if exit_code == 0 { protocol::LogLevel::Info } else { protocol::LogLevel::Error };
            (
                level,
                "arshy",
                serde_json::json!({
                    "event": "task_complete",
                    "task_id": task_id,
                    "status": status,
                    "exit_code": exit_code,
                }),
            )
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
            (
                level,
                "arshy.parser",
                serde_json::json!({
                    "event": "diagnostic",
                    "task_id": notif.params["task_id"].as_str().unwrap_or(""),
                    "type": event["type"].as_str().unwrap_or(""),
                    "severity": severity,
                    "message": event["message"].as_str().unwrap_or(""),
                    "location": event.get("location"),
                }),
            )
        }
        "daemon/shutdown" => (
            protocol::LogLevel::Warning,
            "arshy.daemon",
            serde_json::json!({
                "event": "shutdown",
                "reason": notif.params["reason"].as_str().unwrap_or(""),
            }),
        ),
        "notification/overflow" => (
            protocol::LogLevel::Warning,
            "arshy.transport",
            serde_json::json!({
                "event": "notification_overflow",
                "dropped": notif.params["dropped"].as_u64().unwrap_or(0),
                "hint": "Live updates were dropped; query persisted events with arshy_query.",
            }),
        ),
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

/// Convert a daemon connection error into a user-friendly message.
///
/// Instead of exposing raw IPC/IO errors, shows an actionable message
/// that tells the user exactly what to do.
pub(crate) fn format_daemon_error(
    err: arshy_lib::ArshyError,
    cfg: &Config,
) -> arshy_lib::ArshyError {
    let auto_start = cfg.daemon.auto_start;
    let msg = match &err {
        arshy_lib::ArshyError::DaemonUnreachable(inner) => {
            if inner.contains("crash-loop") {
                "arshyd is crash-looping and auto-start has been suppressed.\n\
                 Check the daemon logs: cat ~/.local/share/arshy/daemon.log\n\
                 Then restart: arshy daemon restart"
                    .to_string()
            } else if inner.contains("did not start") {
                if auto_start {
                    "arshyd daemon could not be started automatically.\n\
                     Start it manually: arshy daemon start\n\
                     If the problem persists, check: arshy doctor"
                        .to_string()
                } else {
                    "arshyd daemon is not running.\n\
                     Run `arshy daemon start` to start it."
                        .to_string()
                }
            } else {
                "arshyd daemon is not running or unreachable.\n\
                 Run `arshy daemon start` to start it.\n\
                 If it was recently running, check for crashes: arshy doctor"
                    .to_string()
            }
        }
        arshy_lib::ArshyError::Io(e) => {
            if e.kind() == std::io::ErrorKind::ConnectionRefused {
                "arshyd daemon is not running (connection refused).\n\
                 Run `arshy daemon start` to start it."
                    .to_string()
            } else if e.kind() == std::io::ErrorKind::NotFound
                || e.to_string().contains("No such file")
            {
                "arshyd daemon socket not found — the daemon is not running.\n\
                 Run `arshy daemon start` to start it."
                    .to_string()
            } else {
                format!(
                    "Failed to connect to arshyd daemon: {}\nRun `arshy doctor` for diagnostics.",
                    e
                )
            }
        }
        _ => {
            format!("Cannot connect to arshyd daemon.\nRun `arshy daemon start` to start it.\nError: {}", err)
        }
    };
    arshy_lib::ArshyError::DaemonUnreachable(msg)
}

#[cfg(test)]
pub(crate) fn format_duration(ms: Option<u64>) -> String {
    match ms {
        None => String::new(),
        Some(m) if m < 1000 => format!("{}ms", m),
        Some(m) if m < 60_000 => format!("{:.1}s", m as f64 / 1000.0),
        Some(m) => format!("{:.1}m", m as f64 / 60_000.0),
    }
}

pub(crate) async fn write_json_response<W: tokio::io::AsyncWrite + Unpin, I: serde::Serialize>(
    stdout: &mut BufWriter<W>,
    id: I,
    result: &serde_json::Value,
) -> Result<()> {
    let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let mut json = serde_json::to_vec(&response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}

pub(crate) async fn write_json_error<W: tokio::io::AsyncWrite + Unpin, I: serde::Serialize>(
    stdout: &mut BufWriter<W>,
    id: I,
    code: i64,
    message: &str,
    retryable: bool,
) -> Result<()> {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": { "retryable": retryable }
        }
    });
    let mut json = serde_json::to_vec(&response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}

/// Write a structured MCP error with code + retryable data field.
pub(crate) async fn write_structured_error<I: serde::Serialize>(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: I,
    err: arshy_lib::ArshyError,
) -> Result<()> {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": err.json_rpc_code(),
            "message": err.to_string(),
            "data": { "retryable": err.is_retryable() }
        }
    });
    let mut json = serde_json::to_vec(&response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(())
}

pub(crate) fn notification_to_json(notif: &Notification) -> Option<serde_json::Value> {
    match notif.method.as_str() {
        "task/update" => {
            let task_id = notif.params["task_id"].as_str().unwrap_or("");
            let status = notif.params["status"].as_str().unwrap_or("");
            Some(serde_json::json!({
                "event": "task_update",
                "task_id": task_id,
                "status": status,
            }))
        }
        "task/complete" => {
            let task_id = notif.params["task_id"].as_str().unwrap_or("");
            let status = notif.params["status"].as_str().unwrap_or("failed");
            let exit_code = notif.params["exit_code"].as_i64().unwrap_or(-1);
            Some(serde_json::json!({
                "event": "task_complete",
                "task_id": task_id,
                "status": status,
                "exit_code": exit_code,
            }))
        }
        "diagnostic" => {
            let event = &notif.params["event"];
            Some(serde_json::json!({
                "event": "diagnostic",
                "task_id": notif.params["task_id"].as_str().unwrap_or(""),
                "type": event["type"].as_str().unwrap_or(""),
                "severity": event["severity"].as_str().unwrap_or(""),
                "message": event["message"].as_str().unwrap_or(""),
                "location": event.get("location"),
            }))
        }
        "daemon/shutdown" => Some(serde_json::json!({
            "event": "shutdown",
            "reason": notif.params["reason"].as_str().unwrap_or(""),
        })),
        "notification/overflow" => Some(serde_json::json!({
            "event": "notification_overflow",
            "dropped": notif.params["dropped"].as_u64().unwrap_or(0),
            "hint": "Live updates were dropped; query persisted events with arshy_query.",
        })),
        _ => None,
    }
}
