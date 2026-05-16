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
use arshy_lib::ipc::{self, DaemonConnection, Notification};
use arshy_lib::mcp::{instructions, protocol};
use arshy_lib::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::mpsc;

/// Middleware trait for intercepting request/response/notification flows.
///
/// Reserved for future use: AuditMiddleware, RateLimitMiddleware, AuthMiddleware.
/// Currently the middleware chain is empty (`Vec::new()`).
#[allow(dead_code)]
pub trait Middleware: Send + Sync {
    /// Called before forwarding a request to the daemon.
    fn on_request(&self, _request: &serde_json::Value) -> Result<()> { Ok(()) }
    /// Called before writing a response to stdout.
    fn on_response(&self, _response: &serde_json::Value) -> Result<()> { Ok(()) }
    /// Called before forwarding a notification to the MCP client.
    fn on_notification(&self, _notif: &Notification) -> Result<()> { Ok(()) }
}

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
    let (mut daemon, mut notif_rx) = DaemonConnection::new(stream);

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = BufWriter::new(tokio::io::stdout());
    let mut line = String::new();

    // Notification batching state
    let batch_interval = Duration::from_millis(cfg.notifications.batch_interval_ms);
    let max_batch = cfg.notifications.max_batch_events;
    let mut pending_notifs: Vec<Notification> = Vec::with_capacity(max_batch);
    let mut batch_deadline: Option<tokio::time::Instant> = None;
    let mut request_tasks: HashMap<u64, String> = HashMap::new();

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
                            "tools/list" => handle_tools_list(&mut stdout, id).await,
                            "prompts/list" => handle_prompts_list(&mut stdout, id).await,
                            "prompts/get" => handle_prompts_get(&mut stdout, &request, id).await,
                            "resources/list" => {
                                match handle_resources_list(&mut daemon, &mut stdout, id).await {
                                    Ok(()) => drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await,
                                    Err(e) => Err(e),
                                }
                            }
                            "resources/read" => {
                                match handle_resources_read(&mut daemon, &mut stdout, &request, id).await {
                                    Ok(()) => drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await,
                                    Err(e) => Err(e),
                                }
                            }
                            "tools/call" => {
                                match handle_tool_call(&mut daemon, &mut stdout, &request, id).await {
                                    Ok(maybe_task_id) => {
                                        if let Some(tid) = maybe_task_id {
                                            request_tasks.insert(id, tid);
                                        }
                                        drain_pending(&mut notif_rx, &mut stdout, &mut pending_notifs, max_batch).await
                                    }
                                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                                        tracing::warn!("daemon connection lost, attempting reconnect...");
                                        match reconnect(&cfg, &socket_path).await {
                                            Ok((new_conn, new_notif_rx)) => {
                                                daemon = new_conn;
                                                notif_rx = new_notif_rx;
                                                tracing::info!("reconnected to daemon");
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
                                            Err(re) => {
                                                tracing::error!("reconnect failed: {}", re);
                                                write_structured_error(&mut stdout, id, arshy_lib::ArshyError::DaemonUnreachable(
                                                    format!("connection lost: {}", e))).await
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        write_structured_error(&mut stdout, id, arshy_lib::ArshyError::Ipc(
                                            format!("daemon error: {}", e))).await
                                    }
                                }
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
                            _ => write_structured_error(&mut stdout, id, arshy_lib::ArshyError::Ipc(
                                format!("unknown method: {}", method))).await,
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
        }
    }

    // Flush any remaining batched notifications
    flush_batch(&mut stdout, &mut pending_notifs).await?;
    Ok(())
}

/// Drain pending notifications from the channel (non-blocking) and forward
/// to stdout. Called after a tool call completes.
async fn drain_pending(
    notif_rx: &mut mpsc::Receiver<Notification>,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    pending: &mut Vec<Notification>,
    max_batch: usize,
) -> Result<()> {
    while let Ok(n) = notif_rx.try_recv() {
        pending.push(n);
        if pending.len() >= max_batch {
            flush_batch(stdout, pending).await?;
        }
    }
    if !pending.is_empty() {
        flush_batch(stdout, pending).await?;
    }
    Ok(())
}

/// Write all pending batched notifications as a single MCP notification message.
async fn flush_batch(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    batch: &mut Vec<Notification>,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }

    if batch.len() == 1 {
        write_mcp_notification(stdout, &batch[0]).await?;
    } else {
        // Coalesce: send as a single notifications/message with a JSON array payload
        let items: Vec<serde_json::Value> = batch.iter()
            .filter_map(notification_to_json)
            .collect();

        if !items.is_empty() {
            let mcp_notif = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/message",
                "params": {
                    "level": "info",
                    "logger": "arshy.batch",
                    "data": {
                        "event": "batch",
                        "count": items.len(),
                        "items": items,
                    }
                }
            });

            let mut json = serde_json::to_vec(&mcp_notif)?;
            json.push(b'\n');
            stdout.write_all(&json).await?;
            stdout.flush().await?;
        }
    }

    batch.clear();
    Ok(())
}

/// Convert a daemon notification to a JSON value for batching.
fn notification_to_json(notif: &Notification) -> Option<serde_json::Value> {
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
            let exit_code = notif.params["exit_code"].as_i64().unwrap_or(-1);
            Some(serde_json::json!({
                "event": "task_complete",
                "task_id": task_id,
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
        "daemon/shutdown" => {
            Some(serde_json::json!({
                "event": "shutdown",
                "reason": notif.params["reason"].as_str().unwrap_or(""),
            }))
        }
        _ => None,
    }
}

// ── MCP handlers ────────────────────────────────────────────────────────────

async fn handle_initialize<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &mut BufWriter<W>,
    id: u64,
    request: &serde_json::Value,
) -> Result<()> {
    const SERVER_VERSION: &str = "2024-11-05";

    // Validate protocol version (backward-compatible: omit = accept)
    let client_version = request["params"]["protocolVersion"]
        .as_str()
        .unwrap_or(SERVER_VERSION);
    let major_client = &client_version[..7.min(client_version.len())];
    let major_server = &SERVER_VERSION[..7];
    if major_client != major_server {
        return write_json_error(
            stdout,
            id,
            ipc::error_code::INVALID_REQUEST,
            &format!(
                "Unsupported protocol version: {} (server supports {})",
                client_version, SERVER_VERSION
            ),
        )
        .await;
    }

    let mut notifications = HashMap::new();
    notifications.insert("diagnostic".to_string(), serde_json::Value::Object(Default::default()));

    let mut resources = HashMap::new();
    resources.insert("listChanged".to_string(), serde_json::json!(true));

    let mut prompts = HashMap::new();
    prompts.insert("listChanged".to_string(), serde_json::json!(true));

    let caps = protocol::ServerCapabilities {
        protocol_version: SERVER_VERSION.into(),
        server_info: protocol::ServerInfo {
            name: "arshy".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        capabilities: protocol::ServerFeatures {
            tools: HashMap::new(),
            resources,
            notifications,
            prompts,
        },
        instructions: Some(instructions::default_instructions()),
    };
    write_json_response(stdout, id, &serde_json::to_value(&caps)?).await
}

async fn handle_tools_list(stdout: &mut BufWriter<tokio::io::Stdout>, id: u64) -> Result<()> {
    let tools = instructions::tool_definitions();
    write_json_response(stdout, id, &serde_json::json!({ "tools": tools })).await
}

async fn handle_prompts_list<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &mut BufWriter<W>,
    id: u64,
) -> Result<()> {
    let prompts = instructions::prompt_definitions();
    write_json_response(stdout, id, &serde_json::json!({ "prompts": prompts })).await
}

async fn handle_prompts_get<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &mut BufWriter<W>,
    request: &serde_json::Value,
    id: u64,
) -> Result<()> {
    let name = request["params"]["name"].as_str().unwrap_or("");
    let args_raw = request["params"]["arguments"].as_object();
    let args: std::collections::HashMap<String, String> = args_raw
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();

    match instructions::get_prompt(name, &args) {
        Some(messages) => {
            write_json_response(stdout, id, &serde_json::json!({
                "description": format!("Prompt: {}", name),
                "messages": messages,
            })).await
        }
        None => write_json_error(
            stdout,
            id,
            ipc::error_code::INVALID_PARAMS,
            &format!("Unknown prompt: {}", name),
        ).await,
    }
}

async fn handle_tool_call(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    request: &serde_json::Value,
    id: u64,
) -> Result<Option<String>> {
    let tool_name = request["params"]["name"].as_str().unwrap_or("");
    let args = request["params"]["arguments"].clone();
    let ipc_method = mcp_tool_to_ipc_method(tool_name, &args);

    let response = daemon.send_request(ipc_method, args).await?;
    let result = &response.result;

    // Check for IPC-level error
    if result.get("error").is_some() {
        let err_msg = result["error"]["message"].as_str().unwrap_or("unknown error");
        let code = result["error"]["code"].as_i64().unwrap_or(arshy_lib::ipc::error_code::INTERNAL_ERROR);
        write_json_error(stdout, id, code, err_msg).await?;
        return Ok(None);
    }

    // Extract task_id for cancellation tracking
    let task_id = result["task_id"].as_str().map(String::from);

    // Short command → plain text (like a native shell)
    let is_short = result["short_command"].as_bool().unwrap_or(false);
    let content = if is_short {
        let raw = result["raw_output"].as_str().unwrap_or("");
        serde_json::json!([{"type": "text", "text": raw}])
    } else {
        // Long command → structured output
        let text = serde_json::to_string_pretty(result).unwrap_or_default();
        serde_json::json!([{"type": "text", "text": text}])
    };

    let mcp_response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "content": content }
    });

    let mut json = serde_json::to_vec(&mcp_response)?;
    json.push(b'\n');
    stdout.write_all(&json).await?;
    stdout.flush().await?;
    Ok(task_id)
}

// ── MCP resource handlers ───────────────────────────────────────────────────

async fn handle_resources_list(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: u64,
) -> Result<()> {
    let response = daemon.send_request(ipc::METHOD_LIST, serde_json::json!({"limit": 50})).await?;
    let tasks = response.result.as_array().cloned().unwrap_or_default();

    let resources: Vec<protocol::ResourceDefinition> = tasks.iter().map(|t| {
        let task_id = t["task_id"].as_str().unwrap_or("");
        let command = t["command"].as_str().unwrap_or("");
        let status = t["status"].as_str().unwrap_or("unknown");
        protocol::ResourceDefinition {
            uri: format!("arshy://task/{}", task_id),
            name: format!("{} [{}]", command, status),
            description: Some(format!("Task {} — {}", task_id, command)),
            mime_type: Some("application/json".into()),
        }
    }).collect();

    write_json_response(stdout, id, &serde_json::json!({ "resources": resources })).await
}

async fn handle_resources_read(
    daemon: &mut DaemonConnection,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    request: &serde_json::Value,
    id: u64,
) -> Result<()> {
    let uri = request["params"]["uri"].as_str().unwrap_or("");
    let task_id = uri.strip_prefix("arshy://task/").unwrap_or("");

    if task_id.is_empty() {
        write_json_error(stdout, id, ipc::error_code::INVALID_PARAMS, "invalid resource URI").await?;
        return Ok(());
    }

    // Get task details via list (with task_id filter not available, query directly)
    let query_resp = daemon.send_request(ipc::METHOD_QUERY, serde_json::json!({
        "task_id": task_id,
        "limit": 200,
    })).await?;

    let events = query_resp.result["events"].as_array().cloned().unwrap_or_default();
    let total = query_resp.result["total"].as_u64().unwrap_or(0);

    let content = serde_json::json!({
        "task_id": task_id,
        "total_events": total,
        "events": events,
    });

    let text = serde_json::to_string_pretty(&content).unwrap_or_default();

    let resource_content = protocol::ResourceContent {
        uri: uri.to_string(),
        mime_type: Some("application/json".into()),
        text,
    };

    write_json_response(stdout, id, &serde_json::json!({
        "contents": [resource_content],
    })).await
}

// ── Daemon notification → MCP notification mapping ──────────────────────────

/// Forward a daemon IPC notification as an MCP `notifications/message`.
async fn write_mcp_notification<W: tokio::io::AsyncWriteExt + Unpin>(
    stdout: &mut BufWriter<W>,
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
) -> Result<(DaemonConnection, mpsc::Receiver<Notification>)> {
    let stream = connect_or_start(cfg, socket_path).await?;
    Ok(DaemonConnection::new(stream))
}

/// Map MCP tool name to IPC method.
///
/// 2-tool model:
/// - `arshy_exec` → action-based dispatch (run/kill/list/tail)
/// - `arshy_query` → structured event queries
fn mcp_tool_to_ipc_method(tool_name: &str, args: &serde_json::Value) -> &'static str {
    match tool_name {
        "arshy_exec" => match args.get("action").and_then(|v| v.as_str()) {
            Some("kill") => ipc::METHOD_KILL,
            Some("list") => ipc::METHOD_LIST,
            Some("tail") => ipc::METHOD_TAIL,
            _ => ipc::METHOD_RUN, // "run" is default for arshy_exec
        },
        "arshy_query" => ipc::METHOD_QUERY,
        // Legacy tool names — still supported for backward compatibility
        "arshy_run" => ipc::METHOD_RUN,
        "arshy_kill" => ipc::METHOD_KILL,
        "arshy_list" => ipc::METHOD_LIST,
        "arshy_tail" => ipc::METHOD_TAIL,
        _ => ipc::METHOD_RUN,
    }
}

async fn write_json_response<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &mut BufWriter<W>,
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

async fn write_json_error<W: tokio::io::AsyncWrite + Unpin>(
    stdout: &mut BufWriter<W>,
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

/// Write a structured MCP error with code + retryable data field.
async fn write_structured_error(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_mcp_tool_to_ipc_method_legacy() {
        let empty = serde_json::json!({});
        assert_eq!(mcp_tool_to_ipc_method("arshy_run", &empty), ipc::METHOD_RUN);
        assert_eq!(mcp_tool_to_ipc_method("arshy_query", &empty), ipc::METHOD_QUERY);
        assert_eq!(mcp_tool_to_ipc_method("arshy_list", &empty), ipc::METHOD_LIST);
        assert_eq!(mcp_tool_to_ipc_method("arshy_kill", &empty), ipc::METHOD_KILL);
        assert_eq!(mcp_tool_to_ipc_method("arshy_tail", &empty), ipc::METHOD_TAIL);
        // Unknown tools default to run
        assert_eq!(mcp_tool_to_ipc_method("unknown", &empty), ipc::METHOD_RUN);
        assert_eq!(mcp_tool_to_ipc_method("", &empty), ipc::METHOD_RUN);
    }

    #[test]
    fn test_mcp_tool_to_ipc_method_arshy_exec_by_action() {
        let run_args = serde_json::json!({"action":"run","command":"ls"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &run_args), ipc::METHOD_RUN);

        let kill_args = serde_json::json!({"action":"kill","task_id":"abc"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &kill_args), ipc::METHOD_KILL);

        let list_args = serde_json::json!({"action":"list"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &list_args), ipc::METHOD_LIST);

        let tail_args = serde_json::json!({"action":"tail","task_id":"abc"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &tail_args), ipc::METHOD_TAIL);

        // Default (no action) → run
        let default_args = serde_json::json!({"command":"ls"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &default_args), ipc::METHOD_RUN);
    }

    #[test]
    fn test_is_connection_error() {
        // Connection closed
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("connection closed".into())));
        // Timed out
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("request timed out".into())));
        // Channel dropped
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("response channel dropped".into())));
        // Broken pipe
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("broken pipe".into())));
        // Connection refused
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("Connection refused".into())));
        // No such file
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("No such file or directory".into())));

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
    async fn test_initialize_mismatched_version_rejected() {
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
        assert_eq!(parsed["error"]["code"], ipc::error_code::INVALID_REQUEST);
        assert!(parsed["error"]["message"].as_str().unwrap().contains("Unsupported protocol version"));
    }

    #[tokio::test]
    async fn test_initialize_advertises_prompts_capability() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({ "params": {} });
        handle_initialize(&mut buf, 4, &request).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert!(parsed["result"]["capabilities"]["prompts"].is_object());
    }

    // ── N3: MCP prompts ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_prompts_list_returns_three() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        handle_prompts_list(&mut buf, 10).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        let prompts = parsed["result"]["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 3);
        assert_eq!(prompts[0]["name"], "analyze_build_failure");
        assert_eq!(prompts[1]["name"], "diagnose_test_failure");
        assert_eq!(prompts[2]["name"], "review_task_output");
    }

    #[tokio::test]
    async fn test_prompts_get_build_failure() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({
            "params": {
                "name": "analyze_build_failure",
                "arguments": { "command": "cargo build" }
            }
        });
        handle_prompts_get(&mut buf, &request, 11).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["id"], 11);
        let messages = parsed["result"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert!(messages[0]["content"]["text"].as_str().unwrap().contains("cargo build"));
    }

    #[tokio::test]
    async fn test_prompts_get_unknown_returns_error() {
        use tokio::io::AsyncWriteExt;
        let mut buf = tokio::io::BufWriter::new(Vec::new());
        let request = serde_json::json!({
            "params": { "name": "nonexistent_prompt" }
        });
        handle_prompts_get(&mut buf, &request, 12).await.unwrap();
        buf.flush().await.unwrap();

        let output = String::from_utf8(buf.into_inner()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["error"]["code"], ipc::error_code::INVALID_PARAMS);
        assert!(parsed["error"]["message"].as_str().unwrap().contains("Unknown prompt"));
    }
}
