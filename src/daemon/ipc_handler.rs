//! Daemon IPC handler — bidirectional JSON-RPC over UDS.
//!
//! Architecture:
//! - Reader task: reads requests from proxy, dispatches, sends responses via channel
//! - Writer task: drains channel (responses + notifications) to the socket
//! - EventBus notifications are forwarded to all connected proxies

use arshy_lib::ipc::{
    self, ErrorResponse, JsonRpcError, Notification, QueryParams, Request, Response,
    RunTaskParams, METHOD_KILL, METHOD_LIST, METHOD_PRUNE, METHOD_QUERY, METHOD_RUN,
    METHOD_STATUS, METHOD_TAIL,
};
use arshy_lib::Result;
use serde::Serialize;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::bus::{BusEvent, EventBus};
use super::exec::Executor;
use super::store::Store;

// ── Outbound message channel ────────────────────────────────────────────────

/// Items the writer task can send to the proxy.
enum Outbound {
    Response(Response),
    Notification(Notification),
}

/// Result returned from the executor after scheduling a task.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub task_id: String,
    pub status: arshy_lib::ipc::TaskStatus,
    pub pid: Option<u32>,
}

// ── Main handler entry ──────────────────────────────────────────────────────

/// Handle a single UDS connection with bidirectional communication.
///
/// Spawns two tasks:
/// - **reader**: reads JSON-RPC requests, dispatches, sends responses via channel
/// - **writer**: drains channel (responses + notifications) to the socket
///
/// EventBus notifications are forwarded to all connected proxies.
pub async fn handle(
    stream: UnixStream,
    conn_id: u64,
    executor: Arc<Executor>,
    store: Arc<Store>,
    event_bus: EventBus,
) -> Result<()> {
    let (reader_half, writer_half) = stream.into_split();
    let mut reader = BufReader::new(reader_half);
    let mut writer = BufWriter::new(writer_half);

    let (tx, rx) = mpsc::channel::<Outbound>(256);

    // Spawn writer task
    let writer_handle = tokio::spawn(async move {
        if let Err(e) = drain_outbound(&mut writer, rx).await {
            tracing::error!("conn {} writer error: {}", conn_id, e);
        }
    });

    // Subscribe to EventBus and forward notifications
    let mut bus_rx = event_bus.subscribe();
    let tx_notif = tx.clone();
    let notif_handle = tokio::spawn(async move {
        while let Ok(event) = bus_rx.recv().await {
            if let Some(notification) = bus_event_to_notification(&event) {
                if tx_notif.send(Outbound::Notification(notification)).await.is_err() {
                    break; // writer dropped
                }
            }
        }
    });

    // Reader loop
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }

        let request: Request = match serde_json::from_str(line.trim()) {
            Ok(r) => r,
            Err(e) => {
                let err = ErrorResponse {
                    jsonrpc: "2.0".into(),
                    id: 0,
                    error: JsonRpcError { code: -32700, message: e.to_string() },
                };
                let _ = tx.send(Outbound::Response(Response {
                    jsonrpc: "2.0".into(),
                    id: 0,
                    result: serde_json::to_value(&err).unwrap_or_default(),
                })).await;
                continue;
            }
        };

        let response = dispatch(&request, &executor, &store).await;
        let outbound = match response {
            Ok(result) => Outbound::Response(Response {
                jsonrpc: "2.0".into(),
                id: request.id,
                result,
            }),
            Err(e) => Outbound::Response(Response {
                jsonrpc: "2.0".into(),
                id: request.id,
                result: serde_json::to_value(&ErrorResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    error: JsonRpcError { code: -32603, message: e.to_string() },
                }).unwrap_or_default(),
            }),
        };

        if tx.send(outbound).await.is_err() {
            break;
        }
    }

    // Cleanup: drop the sender, wait for writer to flush
    drop(tx);
    notif_handle.abort();
    let _ = writer_handle.await;

    tracing::debug!("conn {} closed", conn_id);
    Ok(())
}

// ── Request dispatch ────────────────────────────────────────────────────────

async fn dispatch(request: &Request, executor: &Executor, store: &Store) -> Result<serde_json::Value> {
    match request.method.as_str() {
        METHOD_RUN => {
            let params: RunTaskParams = serde_json::from_value(request.params.clone())?;
            let result = executor
                .run(&params.command, params.cwd.as_deref(), params.timeout_ms, &params.mode)
                .await?;
            Ok(serde_json::to_value(&result)?)
        }
        METHOD_QUERY => {
            let params: QueryParams = serde_json::from_value(request.params.clone())?;
            let (events, total) = store.query_events(&params)?;
            Ok(serde_json::json!({ "events": events, "total": total }))
        }
        METHOD_LIST => {
            let status: Option<String> = request
                .params.get("status").and_then(|v| v.as_str()).map(String::from);
            let limit: usize = request
                .params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let tasks = store.list_tasks(status.as_deref(), limit)?;
            Ok(serde_json::to_value(&tasks)?)
        }
        METHOD_KILL => {
            let task_id = request.params.get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| arshy_lib::ArshyError::Ipc("missing task_id".into()))?;
            executor.kill(task_id).await?;
            Ok(serde_json::json!({ "task_id": task_id, "status": "killed" }))
        }
        METHOD_TAIL => {
            let task_id = request.params.get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| arshy_lib::ArshyError::Ipc("missing task_id".into()))?;
            let lines: usize = request.params.get("lines")
                .and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let format: String = request.params.get("format")
                .and_then(|v| v.as_str()).unwrap_or("event").to_string();
            let output = executor.tail(task_id, lines, &format).await?;
            Ok(serde_json::json!({ "task_id": task_id, "lines": output }))
        }
        METHOD_STATUS => {
            let all = store.list_tasks(None, 0)?;
            let running = store.list_tasks(Some("running"), 0)?.len();
            Ok(serde_json::json!({
                "uptime_secs": daemon_uptime_secs(),
                "tasks_running": running,
                "tasks_total": all.len(),
            }))
        }
        METHOD_PRUNE => {
            let keep = request.params.get("keep")
                .and_then(|v| v.as_u64()).map(|v| v as usize);
            let older_than = request.params.get("older_than")
                .and_then(|v| v.as_u64()).map(|v| v as u32);
            let (td, ed) = if let Some(k) = keep {
                store.prune_keep(k)?
            } else if let Some(d) = older_than {
                store.prune_older_than(d)?
            } else {
                store.prune_keep(1000)?
            };
            Ok(serde_json::json!({ "tasks_deleted": td, "events_deleted": ed }))
        }
        _ => Err(arshy_lib::ArshyError::Ipc(format!(
            "unknown method: {}", request.method
        ))),
    }
}

// ── Writer task ─────────────────────────────────────────────────────────────

/// Drain the outbound channel, writing each item as a JSON line to the socket.
async fn drain_outbound(
    writer: &mut BufWriter<tokio::net::unix::OwnedWriteHalf>,
    mut rx: mpsc::Receiver<Outbound>,
) -> Result<()> {
    while let Some(item) = rx.recv().await {
        match item {
            Outbound::Response(resp) => ipc::write_json_line(writer, &resp).await?,
            Outbound::Notification(notif) => ipc::write_json_line(writer, &notif).await?,
        }
    }
    Ok(())
}

// ── EventBus → IPC notification mapping ─────────────────────────────────────

/// Convert a bus event to a JSON-RPC notification for the proxy.
fn bus_event_to_notification(event: &BusEvent) -> Option<Notification> {
    use super::bus::router::NotificationRouter;
    NotificationRouter::to_notification(event)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn daemon_uptime_secs() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs()
}
