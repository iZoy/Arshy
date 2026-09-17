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
use arshy_lib::ipc::{self, DaemonConnection, Notification, Response};
use arshy_lib::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};
use tokio::sync::mpsc;

fn request_id_key(id: &serde_json::Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| "null".into())
}

async fn recv_notification(
    notif_rx: &mut Option<mpsc::Receiver<Notification>>,
) -> Option<Notification> {
    match notif_rx.as_mut() {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

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
    is_connection_error, is_request_timeout, start_daemon,
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

async fn ensure_daemon_ready(
    daemon: &mut Option<arshy_lib::ipc::DaemonConnection>,
    notif_rx: &mut Option<mpsc::Receiver<Notification>>,
    cfg: &Config,
    socket_path: &std::path::Path,
    stdout: &mut BufWriter<tokio::io::Stdout>,
    pending_notifs: &mut Vec<Notification>,
) -> Result<()> {
    if daemon.is_some() && notif_rx.is_some() {
        return Ok(());
    }

    ensure_daemon_up(daemon, notif_rx, cfg, socket_path, stdout, pending_notifs).await
}

fn append_notification_overflow_notice(
    daemon: Option<&DaemonConnection>,
    pending_notifs: &mut Vec<Notification>,
) {
    let Some(daemon) = daemon else { return };
    append_notification_overflow_count(&daemon.notification_drops_handle(), pending_notifs);
}

fn append_notification_overflow_count(
    notification_drops: &std::sync::atomic::AtomicU64,
    pending_notifs: &mut Vec<Notification>,
) {
    let dropped = notification_drops.swap(0, std::sync::atomic::Ordering::AcqRel);
    if dropped > 0 {
        pending_notifs.push(Notification {
            jsonrpc: "2.0".into(),
            method: ipc::NOTIF_NOTIFICATION_OVERFLOW.into(),
            params: serde_json::json!({ "dropped": dropped }),
        });
    }
}

pub(crate) struct NotificationPump<'a> {
    notif_rx: &'a mut mpsc::Receiver<Notification>,
    pending_notifs: &'a mut Vec<Notification>,
    batch_deadline: &'a mut Option<tokio::time::Instant>,
    batch_interval: Duration,
    max_batch: usize,
}

impl<'a> NotificationPump<'a> {
    pub(crate) fn new(
        notif_rx: &'a mut mpsc::Receiver<Notification>,
        pending_notifs: &'a mut Vec<Notification>,
        batch_deadline: &'a mut Option<tokio::time::Instant>,
        batch_interval: Duration,
        max_batch: usize,
    ) -> Self {
        Self { notif_rx, pending_notifs, batch_deadline, batch_interval, max_batch }
    }

    /// Await one daemon response while continuing to drain live notifications.
    /// This keeps a burst of parser events from blocking the response behind the
    /// bounded notification queue.
    pub(crate) async fn send_request(
        &mut self,
        daemon: &mut DaemonConnection,
        stdout: &mut BufWriter<tokio::io::Stdout>,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<Response> {
        let notification_drops = daemon.notification_drops_handle();
        let mut response = Box::pin(daemon.send_request_with_timeout(method, params, timeout));
        let mut notifications_open = true;

        loop {
            if !notifications_open {
                let result = response.await;
                self.finish_request(stdout, &notification_drops).await?;
                return result;
            }

            if let Some(deadline) = *self.batch_deadline {
                tokio::select! {
                    biased;
                    result = &mut response => {
                        self.finish_request(stdout, &notification_drops).await?;
                        return result;
                    }
                    notification = self.notif_rx.recv() => {
                        match notification {
                            Some(notification) => {
                                self.pending_notifs.push(notification);
                                if self.pending_notifs.len() >= self.max_batch {
                                    flush_batch(stdout, self.pending_notifs).await?;
                                    *self.batch_deadline = None;
                                } else {
                                    *self.batch_deadline = Some(tokio::time::Instant::now() + self.batch_interval);
                                }
                            }
                            None => notifications_open = false,
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        flush_batch(stdout, self.pending_notifs).await?;
                        *self.batch_deadline = None;
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    result = &mut response => {
                        self.finish_request(stdout, &notification_drops).await?;
                        return result;
                    }
                    notification = self.notif_rx.recv() => {
                        match notification {
                            Some(notification) => {
                                self.pending_notifs.push(notification);
                                if self.pending_notifs.len() >= self.max_batch {
                                    flush_batch(stdout, self.pending_notifs).await?;
                                } else {
                                    *self.batch_deadline = Some(tokio::time::Instant::now() + self.batch_interval);
                                }
                            }
                            None => notifications_open = false,
                        }
                    }
                }
            }
        }
    }

    async fn finish_request(
        &mut self,
        stdout: &mut BufWriter<tokio::io::Stdout>,
        notification_drops: &std::sync::atomic::AtomicU64,
    ) -> Result<()> {
        append_notification_overflow_count(notification_drops, self.pending_notifs);
        if !self.pending_notifs.is_empty() {
            flush_batch(stdout, self.pending_notifs).await?;
            *self.batch_deadline = None;
        }
        Ok(())
    }
}

pub(crate) fn daemon_request_timeout(method: &str, params: &serde_json::Value) -> Duration {
    if method == ipc::METHOD_RUN && params["mode"].as_str().unwrap_or("auto") == "auto" {
        Duration::from_secs(75)
    } else {
        Duration::from_secs(60)
    }
}

fn retryable_tool_call(request: &serde_json::Value) -> bool {
    match request["params"]["name"].as_str() {
        Some("arshy_query") => true,
        Some("arshy_task") => {
            matches!(request["params"]["arguments"]["action"].as_str(), Some("list" | "raw"))
        }
        _ => false,
    }
}

async fn write_uncertain_tool_error(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: &serde_json::Value,
    request: &serde_json::Value,
) -> Result<()> {
    let tool = request["params"]["name"].as_str().unwrap_or("tool call");
    let message = if tool == "arshy_exec" {
        "arshyd connection lost while running arshy_exec; the command result is unknown and was not retried. Inspect task history before rerunning it."
    } else {
        "arshyd connection lost while processing the request; the operation result is unknown and was not retried."
    };
    write_structured_error(stdout, id, arshy_lib::ArshyError::Ipc(message.into())).await
}

async fn write_request_timeout_error(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    id: &serde_json::Value,
    error: &arshy_lib::ArshyError,
) -> Result<()> {
    write_structured_error(
        stdout,
        id,
        arshy_lib::ArshyError::Ipc(format!(
            "arshyd request timed out: {}; the operation result may be unknown and was not retried.",
            error
        )),
    )
    .await
}

/// Run the proxy on the caller's Tokio runtime.
pub async fn run_async(config_path: Option<PathBuf>) -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides { config_path, ..Default::default() })?;

    let socket_path = cfg.daemon.expanded_socket_path();

    // Retry connection on startup — daemon may be restarting (launchd KeepAlive).
    // 5 attempts with auto-cd validation ensures the connection is alive.
    let (initial_daemon, initial_notif_rx) =
        connect_with_retry(&cfg, &socket_path, 5, &[500, 500, 500, 500], true)
            .await
            .map_err(|e| format_daemon_error(e, &cfg))?;
    let mut daemon = Some(initial_daemon);
    let mut notif_rx = Some(initial_notif_rx);

    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = BufWriter::new(tokio::io::stdout());
    let mut line = String::new();

    // Notification batching state
    let batch_interval = Duration::from_millis(cfg.notifications.batch_interval_ms);
    let max_batch = cfg.notifications.max_batch_events;
    let mut pending_notifs: Vec<Notification> = Vec::with_capacity(max_batch);
    let mut batch_deadline: Option<tokio::time::Instant> = None;
    let mut request_tasks: HashMap<String, String> = HashMap::new();
    // MCP ids are scoped to one client session. This nonce makes daemon-side
    // replay protection safe when separate proxy processes both start at id 1.
    let proxy_session_id = uuid::Uuid::new_v4().to_string();

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
        append_notification_overflow_notice(daemon.as_ref(), &mut pending_notifs);
        if !pending_notifs.is_empty() && batch_deadline.is_none() {
            batch_deadline = Some(tokio::time::Instant::now() + batch_interval);
        }

        // Build the notification future for select!
        let notif_fut = if pending_notifs.len() >= max_batch {
            // Batch full — flush immediately, then recv
            flush_batch(&mut stdout, &mut pending_notifs).await?;
            batch_deadline = None;
            recv_notification(&mut notif_rx)
        } else if let Some(deadline) = batch_deadline {
            // Have pending notifications — wait for batch interval or more
            tokio::select! {
                n = recv_notification(&mut notif_rx) => {
                    match n {
                        Some(notif) => {
                            pending_notifs.push(notif);
                            batch_deadline = Some(tokio::time::Instant::now() + batch_interval);
                        }
                        None => {
                            daemon = None;
                            notif_rx = None;
                            batch_deadline = None;
                        }
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
            recv_notification(&mut notif_rx)
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

                        let request: serde_json::Value = match serde_json::from_str(line.trim()) {
                            Ok(request) => request,
                            Err(error) => {
                                protocol::write_json_error(
                                    &mut stdout,
                                    serde_json::Value::Null,
                                    -32700,
                                    &format!("parse error: {}", error),
                                    false,
                                )
                                .await?;
                                line.clear();
                                continue;
                            }
                        };

                        let method = request["method"].as_str().unwrap_or("");
                        // JSON-RPC ids may be strings or numbers. Preserve the
                        // exact value so clients can correlate responses and
                        // cancellation never aliases unrelated requests to 0.
                        let id = request.get("id").cloned().unwrap_or(serde_json::Value::Null);

                        let result = match method {
                            "initialize" => handle_initialize(&mut stdout, &id, &request).await,
                            "ping" => write_json_response(&mut stdout, &id, &serde_json::json!({})).await,
                            "tools/list" => handle_tools_list(&mut stdout, &id).await,
                            "resources/list" => {
                                if let Err(e) = ensure_daemon_ready(
                                    &mut daemon,
                                    &mut notif_rx,
                                    &cfg,
                                    &socket_path,
                                    &mut stdout,
                                    &mut pending_notifs,
                                )
                                .await
                                {
                                    write_structured_error(&mut stdout, &id, e).await
                                } else {
                                    let result = {
                                        let mut pump = NotificationPump::new(
                                            notif_rx.as_mut().expect("receiver ready"),
                                            &mut pending_notifs,
                                            &mut batch_deadline,
                                            batch_interval,
                                            max_batch,
                                        );
                                        let result = handle_resources_list(
                                            daemon.as_mut().expect("daemon ready"),
                                            &mut pump,
                                            &mut stdout,
                                            &id,
                                        )
                                        .await;
                                        result
                                    };
                                    match result
                                    {
                                    Ok(()) => drain_pending(notif_rx.as_mut().expect("receiver ready"), &mut stdout, &mut pending_notifs, max_batch).await,
                                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                                        daemon = None;
                                        notif_rx = None;
                                        match ensure_daemon_up(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await {
                                            Ok(()) => {
                                                let result = {
                                                    let mut pump = NotificationPump::new(
                                                        notif_rx.as_mut().expect("receiver ready"),
                                                        &mut pending_notifs,
                                                        &mut batch_deadline,
                                                        batch_interval,
                                                        max_batch,
                                                    );
                                                    let result = handle_resources_list(
                                                        daemon.as_mut().expect("daemon ready"),
                                                        &mut pump,
                                                        &mut stdout,
                                                        &id,
                                                    )
                                                    .await;
                                                    result
                                                };
                                                match result {
                                                Ok(()) => drain_pending(notif_rx.as_mut().expect("receiver ready"), &mut stdout, &mut pending_notifs, max_batch).await,
                                                Err(e2) => Err(e2),
                                                }
                                            }
                                            Err(_) => {
                                                let msg = "arshyd daemon is not running or unreachable.\n\
                                                    Run `arshy daemon start` to start it.".to_string();
                                                write_structured_error(&mut stdout, &id, arshy_lib::ArshyError::DaemonUnreachable(msg)).await
                                            }
                                        }
                                    }
                                    Err(e) => Err(e),
                                    }
                                }
                            }
                            "resources/read" => {
                                if let Err(e) = ensure_daemon_ready(
                                    &mut daemon,
                                    &mut notif_rx,
                                    &cfg,
                                    &socket_path,
                                    &mut stdout,
                                    &mut pending_notifs,
                                )
                                .await
                                {
                                    write_structured_error(&mut stdout, &id, e).await
                                } else {
                                    let result = {
                                        let mut pump = NotificationPump::new(
                                            notif_rx.as_mut().expect("receiver ready"),
                                            &mut pending_notifs,
                                            &mut batch_deadline,
                                            batch_interval,
                                            max_batch,
                                        );
                                        let result = handle_resources_read(
                                            daemon.as_mut().expect("daemon ready"),
                                            &mut pump,
                                            &mut stdout,
                                            &request,
                                            &id,
                                        )
                                        .await;
                                        result
                                    };
                                    match result
                                    {
                                    Ok(()) => drain_pending(notif_rx.as_mut().expect("receiver ready"), &mut stdout, &mut pending_notifs, max_batch).await,
                                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                                        daemon = None;
                                        notif_rx = None;
                                        match ensure_daemon_up(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await {
                                            Ok(()) => {
                                                let result = {
                                                    let mut pump = NotificationPump::new(
                                                        notif_rx.as_mut().expect("receiver ready"),
                                                        &mut pending_notifs,
                                                        &mut batch_deadline,
                                                        batch_interval,
                                                        max_batch,
                                                    );
                                                    let result = handle_resources_read(
                                                        daemon.as_mut().expect("daemon ready"),
                                                        &mut pump,
                                                        &mut stdout,
                                                        &request,
                                                        &id,
                                                    )
                                                    .await;
                                                    result
                                                };
                                                match result {
                                                Ok(()) => drain_pending(notif_rx.as_mut().expect("receiver ready"), &mut stdout, &mut pending_notifs, max_batch).await,
                                                Err(e2) => Err(e2),
                                                }
                                            }
                                            Err(_) => {
                                                let msg = "arshyd daemon is not running or unreachable.\n\
                                                    Run `arshy daemon start` to start it.".to_string();
                                                write_structured_error(&mut stdout, &id, arshy_lib::ArshyError::DaemonUnreachable(msg)).await
                                            }
                                        }
                                    }
                                    Err(e) => Err(e),
                                    }
                                }
                            }
                            "tools/call" => {
                                let result = if let Err(e) = ensure_daemon_ready(
                                    &mut daemon,
                                    &mut notif_rx,
                                    &cfg,
                                    &socket_path,
                                    &mut stdout,
                                    &mut pending_notifs,
                                )
                                .await
                                {
                                    write_structured_error(&mut stdout, &id, e).await
                                } else {
                                let result = {
                                    let mut pump = NotificationPump::new(
                                        notif_rx.as_mut().expect("receiver ready"),
                                        &mut pending_notifs,
                                        &mut batch_deadline,
                                        batch_interval,
                                        max_batch,
                                    );
                                    let result = handle_tool_call(
                                        daemon.as_mut().expect("daemon ready"),
                                        &mut pump,
                                        &mut stdout,
                                        &request,
                                        &id,
                                        &proxy_session_id,
                                    )
                                    .await;
                                    result
                                };
                                match result {
                                    Ok(maybe_task_id) => {
                                        if let Some(tid) = maybe_task_id {
                                            request_tasks.insert(request_id_key(&id), tid);
                                        }
                                        drain_pending(notif_rx.as_mut().expect("receiver ready"), &mut stdout, &mut pending_notifs, max_batch).await
                                    }
                                    Err(e) if is_request_timeout(&e) => {
                                        write_request_timeout_error(&mut stdout, &id, &e).await
                                    }
                                    Err(e) if is_connection_error(&e) && cfg.daemon.auto_start => {
                                        tracing::warn!("daemon connection lost, attempting reconnect...");
                                        daemon = None;
                                        notif_rx = None;
                                        if retryable_tool_call(&request) {
                                        match ensure_daemon_up(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await {
                                            Ok(()) => {
                                                let result = {
                                                    let mut pump = NotificationPump::new(
                                                        notif_rx.as_mut().expect("receiver ready"),
                                                        &mut pending_notifs,
                                                        &mut batch_deadline,
                                                        batch_interval,
                                                        max_batch,
                                                    );
                                                    let result = handle_tool_call(
                                                        daemon.as_mut().expect("daemon ready"),
                                                        &mut pump,
                                                        &mut stdout,
                                                        &request,
                                                        &id,
                                                        &proxy_session_id,
                                                    )
                                                    .await;
                                                    result
                                                };
                                                match result {
                                                    Ok(maybe_task_id) => {
                                                        if let Some(tid) = maybe_task_id {
                                                            request_tasks.insert(request_id_key(&id), tid);
                                                        }
                                                        drain_pending(notif_rx.as_mut().expect("receiver ready"), &mut stdout, &mut pending_notifs, max_batch).await
                                                    }
                                                    Err(e2) => Err(e2),
                                                }
                                            }
                                            Err(_) => {
                                                let msg = "arshyd daemon is not running or unreachable.\n\
                                                    Run `arshy daemon start` to start it.\n\
                                                    Connection failed after retries.".to_string();
                                                write_structured_error(&mut stdout, &id, arshy_lib::ArshyError::DaemonUnreachable(msg)).await
                                            }
                                        }
                                        } else {
                                            let _ = ensure_daemon_up(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await;
                                            write_uncertain_tool_error(&mut stdout, &id, &request).await
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
                                        write_structured_error(&mut stdout, &id, arshy_lib::ArshyError::Ipc(msg)).await
                                    }
                                }
                                };
                                result
                            }
                            "notifications/initialized" => Ok(()),
                            "notifications/cancelled" => {
                                // MCP client cancelled a request — kill the associated task
                                let request_id = request["params"]["requestId"].clone();
                                if let Some(task_id) = request_tasks.remove(&request_id_key(&request_id)) {
                                    tracing::info!("cancelling task {} (request {})", task_id, request_id);
                                    let kill_params = serde_json::json!({ "task_id": &task_id });
                                    if ensure_daemon_ready(&mut daemon, &mut notif_rx, &cfg, &socket_path, &mut stdout, &mut pending_notifs).await.is_err() {
                                        tracing::warn!("cancellation result unknown: daemon is unavailable");
                                        Ok(())
                                    } else {
                                    let result = {
                                        let mut pump = NotificationPump::new(
                                            notif_rx.as_mut().expect("receiver ready"),
                                            &mut pending_notifs,
                                            &mut batch_deadline,
                                            batch_interval,
                                            max_batch,
                                        );
                                        let result = pump
                                            .send_request(
                                                daemon.as_mut().expect("daemon ready"),
                                                &mut stdout,
                                                ipc::METHOD_KILL,
                                                kill_params,
                                                Duration::from_secs(60),
                                            )
                                            .await;
                                        result
                                    };
                                    match result {
                                        Ok(_) => {
                                            drain_pending(notif_rx.as_mut().expect("receiver ready"), &mut stdout, &mut pending_notifs, max_batch).await
                                        }
                                        Err(e) => {
                                            daemon = None;
                                            notif_rx = None;
                                            tracing::error!("kill task {} failed: {}", task_id, e);
                                            Ok(())
                                        }
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
                                    write_structured_error(&mut stdout, &id, arshy_lib::ArshyError::Ipc(
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
                    None => {
                        daemon = None;
                        notif_rx = None;
                        batch_deadline = None;
                    }
                }
            }

            // ── Shutdown signal branch ──────────────────────────────────
            // SIGTERM from parent process triggers graceful daemon shutdown
            _ = shutdown_rx.changed() => {
                tracing::info!("shutting down proxy without stopping the shared daemon");
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
    fn non_idempotent_tool_calls_are_not_retryable() {
        assert!(!retryable_tool_call(&serde_json::json!({
            "params": { "name": "arshy_exec", "arguments": { "command": "echo once" } }
        })));
        assert!(!retryable_tool_call(&serde_json::json!({
            "params": { "name": "arshy_task", "arguments": { "action": "cancel" } }
        })));
        assert!(retryable_tool_call(&serde_json::json!({
            "params": { "name": "arshy_query", "arguments": {} }
        })));
        assert!(retryable_tool_call(&serde_json::json!({
            "params": { "name": "arshy_task", "arguments": { "action": "raw" } }
        })));
    }

    #[test]
    fn notification_overflow_is_exposed_as_a_query_hint() {
        let dropped = std::sync::atomic::AtomicU64::new(7);
        let mut pending = Vec::new();

        append_notification_overflow_count(&dropped, &mut pending);

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].method, ipc::NOTIF_NOTIFICATION_OVERFLOW);
        assert_eq!(pending[0].params["dropped"], 7);
        assert_eq!(dropped.load(std::sync::atomic::Ordering::Acquire), 0);
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
        // Legacy one-operation tool names are gone — no compatibility shims.
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
    fn test_mcp_tool_to_ipc_method_single_purpose_tools() {
        let run_args = serde_json::json!({"command":"ls"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_exec", &run_args).unwrap(), ipc::METHOD_RUN);

        let kill_args = serde_json::json!({"action":"cancel","task_id":"abc"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_task", &kill_args).unwrap(), ipc::METHOD_KILL);

        let list_args = serde_json::json!({"action":"list"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_task", &list_args).unwrap(), ipc::METHOD_LIST);

        let raw_args = serde_json::json!({"action":"raw","task_id":"abc"});
        assert_eq!(mcp_tool_to_ipc_method("arshy_task", &raw_args).unwrap(), ipc::METHOD_TAIL);

        assert!(mcp_tool_to_ipc_method("arshy_task", &serde_json::json!({})).is_err());
    }

    #[test]
    fn test_is_connection_error() {
        // Connection closed
        assert!(is_connection_error(&arshy_lib::ArshyError::Ipc("connection closed".into())));
        // Request timeouts are distinct from a closed daemon connection.
        assert!(!is_connection_error(&arshy_lib::ArshyError::Ipc(
            "request 'task/run' timed out after 75s".into()
        )));
        assert!(is_request_timeout(&arshy_lib::ArshyError::Ipc(
            "request 'task/run' timed out after 75s".into()
        )));
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
        let shaped =
            build_query_result(&daemon_result, &serde_json::json!({"limit": 2, "offset": 0}));
        assert!(shaped["content"][0]["text"].as_str().unwrap().contains("\"total\":17"));
        assert_eq!(shaped["structuredContent"]["total"], 17);
        assert_eq!(shaped["structuredContent"]["events"].as_array().unwrap().len(), 2);
        assert_eq!(shaped["structuredContent"]["events"][0]["task_id"], "abc-1");
        assert_eq!(shaped["structuredContent"]["events"][1]["task_id"], "abc-2");
    }

    #[test]
    fn test_build_tail_result_joins_lines_into_content() {
        let daemon_result = serde_json::json!({
            "task_id": "t-42",
            "lines": ["line one", "line two"]
        });
        let shaped = build_tail_result(&daemon_result);
        assert_eq!(shaped["content"][0]["text"], "line one\nline two");
        assert_eq!(shaped["structuredContent"]["task_id"], "t-42");
        assert_eq!(shaped["structuredContent"]["lines"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_build_query_result_robust_to_missing_fields() {
        let shaped = build_query_result(&serde_json::json!({}), &serde_json::json!({}));
        assert_eq!(shaped["structuredContent"]["total"], 0);
        assert_eq!(shaped["structuredContent"]["events"].as_array().unwrap().len(), 0);
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
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].name, "arshy_exec");
        assert_eq!(tools[1].name, "arshy_query");
        assert!(tools[1].description.contains("task_id"));
        assert_eq!(tools[2].name, "arshy_task");
    }
}
