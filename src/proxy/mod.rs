//! MCP stdio proxy — connects MCP clients (Claude Code, Cursor) to the arshy daemon.
//!
//! Architecture:
//! - Uses `tokio::select!` to simultaneously read MCP JSON-RPC from stdin
//!   and forward daemon notifications to stdout in real time
//! - Forwards tool calls to daemon via `DaemonConnection`
//! - Auto-starts daemon if needed
//! - Notification batching: high-frequency events are coalesced within a configurable window
//! - Middleware chain: extensible request/response/notification hooks (currently empty)

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, Notification};
use arshy_lib::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};

/// Middleware trait for intercepting request/response/notification flows.
///
/// Reserved for future use: AuditMiddleware, RateLimitMiddleware, AuthMiddleware.
/// Currently the middleware chain is empty (`Vec::new()`).
#[allow(dead_code)]
mod connection;
mod handlers;
mod protocol;

pub(crate) use connection::{
    connect_or_start, connect_with_retry, drain_pending, ensure_daemon_up, flush_batch,
    is_connection_error, start_daemon,
};
pub(crate) use handlers::{
    handle_initialize, handle_resources_list, handle_resources_read, handle_tool_call,
    handle_tools_list,
};
pub(crate) use protocol::{format_daemon_error, write_json_response, write_structured_error};

/// Middleware trait for intercepting request/response/notification flows.
///
/// Reserved for future use: AuditMiddleware, RateLimitMiddleware, AuthMiddleware.
/// Currently the middleware chain is empty (`Vec::new()`).
#[allow(dead_code)]
pub trait Middleware: Send + Sync {
    /// Called before forwarding a request to the daemon.
    fn on_request(&self, _request: &serde_json::Value) -> Result<()> {
        Ok(())
    }
    /// Called before writing a response to stdout.
    fn on_response(&self, _response: &serde_json::Value) -> Result<()> {
        Ok(())
    }
    /// Called before forwarding a notification to the MCP client.
    fn on_notification(&self, _notif: &Notification) -> Result<()> {
        Ok(())
    }
}

/// Run the MCP stdio proxy. Reads JSON-RPC from stdin, forwards to daemon, writes to stdout.
pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(proxy_main(config_path))
}

async fn proxy_main(config_path: Option<PathBuf>) -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides { config_path, ..Default::default() })?;

    let socket_path = cfg.daemon.expanded_socket_path();

    // Retry connection on startup — daemon may be restarting (launchd KeepAlive).
    // 5 attempts with auto-cd validation ensures the connection is alive.
    let (mut daemon, mut notif_rx) =
        connect_with_retry(&cfg, &socket_path, 5, &[500, 500, 500, 500], true)
            .await
            .map_err(|e| format_daemon_error(e, &cfg))?;

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = BufWriter::new(tokio::io::stdout());
    let mut line = String::new();

    // Notification batching state
    let batch_interval = Duration::from_millis(cfg.notifications.batch_interval_ms);
    let max_batch = cfg.notifications.max_batch_events;
    let mut pending_notifs: Vec<Notification> = Vec::with_capacity(max_batch);
    let mut batch_deadline: Option<tokio::time::Instant> = None;
    let mut request_tasks: HashMap<u64, String> = HashMap::new();

    // Shutdown signal — triggered by SIGTERM from parent process (Claude Code)
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            let _ = sig.recv().await;
            tracing::info!("received SIGTERM, initiating graceful shutdown");
            let _ = shutdown_tx.send(true);
        }
    });

    loop {
        // Build the notification future for select!
        let notif_fut = if pending_notifs.len() >= max_batch {
            // Batch full — flush immediately, then recv
            flush_batch(&mut stdout, &mut pending_notifs).await?;
            batch_deadline = None;
            notif_rx.recv()
        } else if let Some(deadline) = batch_deadline {
            // Have pending notifications — wait for batch interval or more
            tokio::select! {
                n = notif_rx.recv() => {
                    match n {
                        Some(notif) => {
                            pending_notifs.push(notif);
                            batch_deadline = Some(tokio::time::Instant::now() + batch_interval);
                        }
                        None => break, // daemon disconnected
                    }
                    continue;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    flush_batch(&mut stdout, &mut pending_notifs).await?;
                    batch_deadline = None;
                    continue;
                }
            }
        } else {
            notif_rx.recv()
        };

        // Main select: stdin vs notifications
        tokio::select! {
            // ── Stdin branch ────────────────────────────────────────────
            result = stdin.read_line(&mut line) => {
                match result {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        if line.trim().is_empty() {
                            line.clear();
                            continue;
                        }

                        let request: serde_json::Value = serde_json::from_str(line.trim())
                            .map_err(|e| arshy_lib::ArshyError::Mcp(e.to_string()))?;

                        let method = request["method"].as_str().unwrap_or("");
                        let id = request["id"].as_u64().unwrap_or(0);

                        let result = match method {
                            "initialize" => handle_initialize(&mut stdout, id, &request).await,
                            "ping" => write_json_response(&mut stdout, id, &serde_json::json!({})).await,
                            "tools/list" => handle_tools_list(&mut stdout, id).await,
                            "resources/list" => {
                                match handle_resources_list(&mut daemon, &mut stdout, id).await {
                                    Ok(()) => drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await,
                                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                                        match ensure_daemon_up(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await {
                                            Ok(()) => match handle_resources_list(&mut daemon, &mut stdout, id).await {
                                                Ok(()) => drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await,
                                                Err(e2) => Err(e2),
                                            },
                                            Err(_) => {
                                                let msg = "arshyd daemon is not running or unreachable.\n\
                                                    Run `arshy daemon start` to start it.".to_string();
                                                write_structured_error(&mut stdout, id, arshy_lib::ArshyError::DaemonUnreachable(msg)).await
                                            }
                                        }
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            "resources/read" => {
                                match handle_resources_read(&mut daemon, &mut stdout, &request, id).await {
                                    Ok(()) => drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await,
                                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                                        match ensure_daemon_up(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await {
                                            Ok(()) => match handle_resources_read(&mut daemon, &mut stdout, &request, id).await {
                                                Ok(()) => drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await,
                                                Err(e2) => Err(e2),
                                            },
                                            Err(_) => {
                                                let msg = "arshyd daemon is not running or unreachable.\n\
                                                    Run `arshy daemon start` to start it.".to_string();
                                                write_structured_error(&mut stdout, id, arshy_lib::ArshyError::DaemonUnreachable(msg)).await
                                            }
                                        }
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            "tools/call" => {
                                let result = match handle_tool_call(&mut daemon, &mut stdout, &request, id).await {
                                    Ok(maybe_task_id) => {
                                        if let Some(tid) = maybe_task_id {
                                            request_tasks.insert(id, tid);
                                        }
                                        drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await
                                    }
                                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                                        tracing::warn!("daemon connection lost, attempting reconnect...");
                                        match ensure_daemon_up(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await {
                                            Ok(()) => {
                                                match handle_tool_call(&mut daemon, &mut stdout, &request, id).await {
                                                    Ok(maybe_task_id) => {
                                                        if let Some(tid) = maybe_task_id {
                                                            request_tasks.insert(id, tid);
                                                        }
                                                        drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await
                                                    }
                                                    Err(e2) => Err(e2),
                                                }
                                            }
                                            Err(_) => {
                                                let msg = "arshyd daemon is not running or unreachable.\n\
                                                    Run `arshy daemon start` to start it.\n\
                                                    Connection failed after retries.".to_string();
                                                write_structured_error(&mut stdout, id, arshy_lib::ArshyError::DaemonUnreachable(msg)).await
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let msg = match &e {
                                            arshy_lib::ArshyError::DaemonUnreachable(_) => {
                                                "arshyd daemon is not running or unreachable.\n\
                                                 Run `arshy daemon start` to start it.".to_string()
                                            }
                                            arshy_lib::ArshyError::Ipc(inner) if is_connection_error(&e) => {
                                                format!("arshyd daemon connection lost: {}\nRun `arshy daemon start` to restart it.", inner)
                                            }
                                            _ => format!("daemon error: {}", e),
                                        };
                                        write_structured_error(&mut stdout, id, arshy_lib::ArshyError::Ipc(msg)).await
                                    }
                                };
                                result
                            }
                            "notifications/initialized" => Ok(()),
                            "notifications/cancelled" => {
                                // MCP client cancelled a request — kill the associated task
                                let request_id = request["params"]["requestId"].as_u64().unwrap_or(0);
                                if let Some(task_id) = request_tasks.remove(&request_id) {
                                    tracing::info!("cancelling task {} (request {})", task_id, request_id);
                                    let kill_params = serde_json::json!({ "task_id": &task_id });
                                    match daemon.send_request(ipc::METHOD_KILL, kill_params).await {
                                        Ok(_) => {
                                            drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await
                                        }
                                        Err(e) => {
                                            tracing::error!("kill task {} failed: {}", task_id, e);
                                            Ok(())
                                        }
                                    }
                                } else {
                                    tracing::debug!("cancelled unknown request {}", request_id);
                                    Ok(())
                                }
                            }
                            _ => {
                                // Only error on requests (have id), silently ignore notifications
                                if request.get("id").is_some() {
                                    write_structured_error(&mut stdout, id, arshy_lib::ArshyError::Ipc(
                                        format!("unknown method: {}", method))).await
                                } else {
                                    tracing::debug!("ignoring unknown notification: {}", method);
                                    Ok(())
                                }
                            }
                        };

                        if let Err(e) = result {
                            tracing::error!("proxy error processing '{}': {}", method, e);
                        }

                        line.clear();
                    }
                    Err(e) => {
                        tracing::error!("stdin read error: {}", e);
                        break;
                    }
                }
            }

            // ── Notification branch ──────────────────────────────────────
            notif = notif_fut => {
                match notif {
                    Some(n) => {
                        pending_notifs.push(n);
                        if pending_notifs.len() >= max_batch {
                            flush_batch(&mut stdout, &mut pending_notifs).await?;
                            batch_deadline = None;
                        } else {
                            batch_deadline = Some(tokio::time::Instant::now() + batch_interval);
                        }
                    }
                    None => break, // daemon channel closed
                }
            }

            // ── Shutdown signal branch ──────────────────────────────────
            // SIGTERM from parent process triggers graceful daemon shutdown
            _ = shutdown_rx.changed() => {
                tracing::info!("shutting down proxy, forwarding to daemon...");
                // Best-effort: tell daemon to shut down
                let _ = daemon.send_request_with_timeout(
                    ipc::METHOD_SHUTDOWN,
                    serde_json::json!({}),
                    Duration::from_secs(2),
                ).await;
                break;
            }
        }
    }

    // Flush any remaining batched notifications
    flush_batch(&mut stdout, &mut pending_notifs).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::handlers::{build_query_result, build_tail_result, mcp_tool_to_ipc_method};
    use super::protocol::{format_duration, write_mcp_notification};
    use super::*;
    use arshy_lib::mcp::instructions;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(None), "");
        assert_eq!(format_duration(Some(0)), "0ms");
        assert_eq!(format_duration(Some(500)), "500ms");
        assert_eq!(format_duration(Some(999)), "999ms");
        assert_eq!(format_duration(Some(1000)), "1.0s");
        assert_eq!(format_duration(Some(2300)), "2.3s");
        assert_eq!(format_duration(Some(59999)), "60.0s");
        assert_eq!(format_duration(Some(60000)), "1.0m");
        assert_eq!(format_duration(Some(125000)), "2.1m");
    }

    #[test]
    fn test_resource_uri_format() {
        // Resource URIs follow arshy://task/{task_id} format
        let uri = format!("arshy://task/{}", "abc-123");
        assert_eq!(uri, "arshy://task/abc-123");
        let extracted = uri.strip_prefix("arshy://task/").unwrap();
        assert_eq!(extracted, "abc-123");
    }

    #[test]
    fn test_resource_uri_empty_task_id() {
        let uri = "arshy://task/";
        let task_id = uri.strip_prefix("arshy://task/").unwrap();
        assert!(task_id.is_empty());
    }

    #[test]
    fn test_resource_uri_invalid() {
        let uri = "invalid://uri";
        let task_id = uri.strip_prefix("arshy://task/");
        assert!(task_id.is_none());
    }

    #[test]
    fn test_mcp_tool_to_ipc_method_unknown_tool_rejected() {
        let empty = serde_json::json!({});
        // Legacy pre-2-tool names are gone — no compatibility shims.
        assert!(mcp_tool_to_ipc_method("arshy_run", &empty).is_err());
        assert!(mcp_tool_to_ipc_method("arshy_list", &empty).is_err());
        assert!(mcp_tool_to_ipc_method("arshy_kill", &empty).is_err());
        assert!(mcp_tool_to_ipc_method("arshy_tail", &empty).is_err());
        // Unknown tools must error — a typo'd tool name must never silently
        // fall back to running a command.
        assert!(mcp_tool_to_ipc_method("unknown", &empty).is_err());
        assert!(mcp_tool_to_ipc_method("", &empty).is_err());
        assert_eq!(mcp_tool_to_ipc_method("arshy_query", &empty).unwrap(), ipc::METHOD_QUERY);
    }

    #[test]
    fn test_mcp_tool_to_ipc_method_arshy_exec_by_action() {
        let run_args = serde_json::json!({"action":"run","command":"ls"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &run_args).unwrap(), ipc::METHOD_RUN);

        let kill_args = serde_json::json!({"action":"kill","task_id":"abc"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &kill_args).unwrap(), ipc::METHOD_KILL);

        let list_args = serde_json::json!({"action":"list"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &list_args).unwrap(), ipc::METHOD_LIST);

        let tail_args = serde_json::json!({"action":"tail","task_id":"abc"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &tail_args).unwrap(), ipc::METHOD_TAIL);
        let raw_args = serde_json::json!({"action":"raw","task_id":"abc"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &raw_args).unwrap(), ipc::METHOD_TAIL);

        // Default (no action) → run
        let default_args = serde_json::json!({"command":"ls"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &default_args).unwrap(), ipc::METHOD_RUN);
    }

    #[test]
    fn test_is_connection_error() {
        // Connection closed
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("connection closed".into())));
        // Timed out
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("request timed out".into())));
        // Channel dropped
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc(
            "response channel dropped".into()
        )));
        // Broken pipe
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("broken pipe".into())));
        // Connection refused
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("Connection refused".into())));
        // No such file
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc(
            "No such file or directory".into()
        )));

        // Non-connection errors
        assert!(!is_connection_error(&arshy_lib::ArshyError::Ipc("parse error".into())));
        assert!(!is_connection_error(&arshy_lib::ArshyError::Ipc("invalid params".into())));
    }

    #[tokio::test]
    async fn test_write_mcp_notification_task_update() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());

        let notif = Notification {
            jsonrpc: "2.0".into(),
            method: "task/update".into(),
            params: serde_json::json!({"task_id": "abc-123", "status": "running"}),
        };

        write_mcp_notification(&mut buf, &notif).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();

        assert_eq!(parsed["method"], "notifications/message");
        assert_eq!(parsed["params"]["level"], "info");
        assert_eq!(parsed["params"]["logger"], "arshy");
        assert_eq!(parsed["params"]["data"]["event"], "task_update");
        assert_eq!(parsed["params"]["data"]["task_id"], "abc-123");
        assert_eq!(parsed["params"]["data"]["status"], "running");
    }

    #[tokio::test]
    async fn test_write_mcp_notification_task_complete_success() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());

        let notif = Notification {
            jsonrpc: "2.0".into(),
            method: "task/complete".into(),
            params: serde_json::json!({"task_id": "abc-123", "exit_code": 0}),
        };

        write_mcp_notification(&mut buf, &notif).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();

        assert_eq!(parsed["params"]["level"], "info");
        assert_eq!(parsed["params"]["data"]["event"], "task_complete");
        assert_eq!(parsed["params"]["data"]["exit_code"], 0);
    }

    #[tokio::test]
    async fn test_write_mcp_notification_task_complete_failure() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());

        let notif = Notification {
            jsonrpc: "2.0".into(),
            method: "task/complete".into(),
            params: serde_json::json!({"task_id": "abc-123", "exit_code": 1}),
        };

        write_mcp_notification(&mut buf, &notif).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();

        assert_eq!(parsed["params"]["level"], "error");
        assert_eq!(parsed["params"]["data"]["exit_code"], 1);
    }

    #[tokio::test]
    async fn test_write_mcp_notification_diagnostic() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());

        let notif = Notification {
            jsonrpc: "2.0".into(),
            method: "diagnostic".into(),
            params: serde_json::json!({
                "task_id": "abc-123",
                "event": {
                    "type": "compile_error",
                    "severity": "error",
                    "message": "expected semicolon",
                    "location": {"file": "main.rs", "line": 42}
                }
            }),
        };

        write_mcp_notification(&mut buf, &notif).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();

        assert_eq!(parsed["params"]["level"], "error");
        assert_eq!(parsed["params"]["logger"], "arshy.parser");
        assert_eq!(parsed["params"]["data"]["event"], "diagnostic");
        assert_eq!(parsed["params"]["data"]["type"], "compile_error");
        assert_eq!(parsed["params"]["data"]["message"], "expected semicolon");
        assert_eq!(parsed["params"]["data"]["location"]["file"], "main.rs");
    }

    #[tokio::test]
    async fn test_write_mcp_notification_diagnostic_warning() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());

        let notif = Notification {
            jsonrpc: "2.0".into(),
            method: "diagnostic".into(),
            params: serde_json::json!({
                "task_id": "abc-123",
                "event": {
                    "type": "lint",
                    "severity": "warning",
                    "message": "unused variable"
                }
            }),
        };

        write_mcp_notification(&mut buf, &notif).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();

        assert_eq!(parsed["params"]["level"], "warning");
    }

    #[tokio::test]
    async fn test_write_mcp_notification_daemon_shutdown() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());

        let notif = Notification {
            jsonrpc: "2.0".into(),
            method: "daemon/shutdown".into(),
            params: serde_json::json!({"reason": "graceful"}),
        };

        write_mcp_notification(&mut buf, &notif).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();

        assert_eq!(parsed["params"]["level"], "warning");
        assert_eq!(parsed["params"]["logger"], "arshy.daemon");
        assert_eq!(parsed["params"]["data"]["event"], "shutdown");
        assert_eq!(parsed["params"]["data"]["reason"], "graceful");
    }

    #[tokio::test]
    async fn test_write_mcp_notification_unknown_ignored() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());

        let notif = Notification {
            jsonrpc: "2.0".into(),
            method: "unknown/method".into(),
            params: serde_json::json!({}),
        };

        write_mcp_notification(&mut buf, &notif).await.unwrap();
        buf.flush().await.unwrap();

        let output = buf.into_inner();
        // Unknown notifications produce no output
        assert!(output.is_empty());
    }

    // ── N4: Protocol version negotiation ──────────────────────────────────────

    #[tokio::test]
    async fn test_initialize_matching_version() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({
            "params": { "protocolVersion": "2024-11-05" }
        });
        handle_initialize(&mut buf, 1, &request).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(parsed["result"]["serverInfo"]["name"], "arshy");
    }

    #[tokio::test]
    async fn test_initialize_echoes_newer_supported_version() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({
            "params": { "protocolVersion": "2025-11-25" }
        });
        handle_initialize(&mut buf, 11, &request).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], 11);
        // Newer clients (e.g. Claude Code 2.1+) negotiate with 2025-11-25;
        // echo it back instead of answering with a stale hardcoded version.
        assert_eq!(parsed["result"]["protocolVersion"], "2025-11-25");
    }

    #[tokio::test]
    async fn test_initialize_echoes_intermediate_supported_version() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({
            "params": { "protocolVersion": "2025-06-18" }
        });
        handle_initialize(&mut buf, 12, &request).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], 12);
        assert_eq!(parsed["result"]["protocolVersion"], "2025-06-18");
    }

    #[tokio::test]
    async fn test_initialize_missing_version_accepted() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({ "params": {} });
        handle_initialize(&mut buf, 2, &request).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], 2);
        assert_eq!(parsed["result"]["protocolVersion"], "2024-11-05");
    }

    #[tokio::test]
    async fn test_initialize_mismatched_version_responds_with_server_version() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({
            "params": { "protocolVersion": "2025-06-01" }
        });
        handle_initialize(&mut buf, 3, &request).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], 3);
        // Per MCP spec: server responds with its own version, does NOT error
        assert_eq!(parsed["result"]["protocolVersion"], "2024-11-05");
        assert!(parsed["error"].is_null());
    }

    // ── MCP ping ───────────────────────────────────────────────────────────────

    #[test]
    fn test_build_query_result_keeps_events_and_total() {
        let daemon_result = serde_json::json!({
            "events": [
                {"type": "diagnostic", "severity": "error", "message": "x",
                 "task_id": "abc-1"},
                {"type": "diagnostic", "severity": "error", "message": "y",
                 "task_id": "abc-2"}
            ],
            "total": 17
        });
        let shaped = build_query_result(&daemon_result);
        assert_eq!(shaped["content"][0]["text"], "17 events");
        assert_eq!(shaped["total"], 17);
        assert_eq!(shaped["events"].as_array().unwrap().len(), 2);
        assert_eq!(shaped["events"][0]["task_id"], "abc-1");
        assert_eq!(shaped["events"][1]["task_id"], "abc-2");
    }

    #[test]
    fn test_build_tail_result_joins_lines_into_content() {
        let daemon_result = serde_json::json!({
            "task_id": "t-42",
            "lines": ["line one", "line two"]
        });
        let shaped = build_tail_result(&daemon_result);
        assert_eq!(shaped["content"][0]["text"], "line one\nline two");
        assert_eq!(shaped["task_id"], "t-42");
        assert_eq!(shaped["lines"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_build_query_result_robust_to_missing_fields() {
        let shaped = build_query_result(&serde_json::json!({}));
        assert_eq!(shaped["content"][0]["text"], "0 events");
        assert_eq!(shaped["total"], 0);
        assert_eq!(shaped["events"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_ping_returns_empty_result() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        // ping is handled directly in the dispatch loop via write_json_response,
        // so test write_json_response with empty result directly
        write_json_response(&mut buf, 42, &serde_json::json!({})).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], 42);
        assert_eq!(parsed["result"], serde_json::json!({}));
        assert!(parsed["error"].is_null());
    }

    // ── Instructions ──────────────────────────────────────────────────────────

    #[test]
    fn test_instructions_are_plain_string() {
        let instructions = instructions::default_instructions();
        // Must be a plain string, not a JSON object
        assert!(
            serde_json::from_str::<serde_json::Value>(&instructions).is_err(),
            "instructions should be plain text, not valid JSON"
        );
        assert!(instructions.contains("arshy_exec"));
        // The fallback rule must be present (fall back to Bash when unreachable).
        assert!(instructions.contains("unreachable"));
    }

    #[test]
    fn test_tool_definitions_have_descriptions() {
        let tools = instructions::tool_definitions();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "arshy_exec");
        assert!(tools[0].description.contains("Examples:"));
        assert_eq!(tools[1].name, "arshy_query");
        assert!(tools[1].description.contains("task_id"));
    }
}
