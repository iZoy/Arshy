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
use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};
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
    /// Only populated in sync mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_count: Option<u64>,
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

// ── Integration tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arshy_lib::config::ParserConfig;
    use super::super::parser::Engine;
    use arshy_lib::ipc::{DaemonConnection, METHOD_KILL, METHOD_LIST,
        METHOD_PRUNE, METHOD_QUERY, METHOD_RUN, METHOD_STATUS, METHOD_TAIL};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::net::UnixStream;

    /// Full daemon stack: Store + Parser + EventBus + Executor.
    /// Returns a client-side DaemonConnection and the TempDir (must keep alive).
    async fn spawn_daemon_pair() -> (DaemonConnection, Arc<Store>, TempDir) {
        let (client_stream, daemon_stream) = UnixStream::pair().unwrap();
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Arc::new(Store::open(&db_path, false).unwrap());
        store.initialize_schema().unwrap();
        let parser = Arc::new(Engine::new(&ParserConfig::default()).unwrap());
        let bus = EventBus::new();
        let executor = Arc::new(Executor::new(store.clone(), parser, bus.clone()));

        let store_clone = store.clone();
        tokio::spawn(async move {
            let _ = handle(daemon_stream, 0, executor, store_clone, bus).await;
        });

        // Give the daemon side a moment to set up
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        (DaemonConnection::new(client_stream), store, tmp)
    }

    // ── Task 1: Single-method integration tests ────────────────────────────

    #[tokio::test]
    async fn e2e_run_async() {
        let (mut conn, store, _tmp) = spawn_daemon_pair().await;

        let resp = conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "echo hello",
            "mode": "async",
        })).await.unwrap();

        let task_id = resp.result["task_id"].as_str().unwrap();
        assert!(!task_id.is_empty());
        assert_eq!(resp.result["status"], "running");

        // Task should be in the store
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let task = store.get_task(task_id).unwrap().unwrap();
        assert_eq!(task.command, "echo hello");
    }

    #[tokio::test]
    async fn e2e_run_sync() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        let resp = conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "echo sync_test",
            "mode": "sync",
        })).await.unwrap();

        assert_eq!(resp.result["status"], "completed");
        assert_eq!(resp.result["exit_code"], 0);
        assert!(resp.result["duration_ms"].as_u64().unwrap() > 0);
        assert!(resp.result["event_count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn e2e_run_sync_failure() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        let resp = conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "exit 42",
            "mode": "sync",
        })).await.unwrap();

        assert_eq!(resp.result["status"], "failed");
        assert_eq!(resp.result["exit_code"], 42);
    }

    #[tokio::test]
    async fn e2e_query_after_run() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        // Run a task synchronously
        let run_resp = conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "echo query_test",
            "mode": "sync",
        })).await.unwrap();

        let task_id = run_resp.result["task_id"].as_str().unwrap().to_string();

        // Query events
        let query_resp = conn.send_request(METHOD_QUERY, serde_json::json!({
            "task_id": task_id,
            "limit": 100,
        })).await.unwrap();

        let total = query_resp.result["total"].as_u64().unwrap();
        assert!(total >= 1, "expected at least 1 event, got {}", total);

        let events = query_resp.result["events"].as_array().unwrap();
        assert!(!events.is_empty());
        assert!(events[0]["message"].as_str().unwrap().contains("query_test"));
    }

    #[tokio::test]
    async fn e2e_list_tasks() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        // Run a couple of tasks
        conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "echo a", "mode": "sync",
        })).await.unwrap();

        conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "echo b", "mode": "sync",
        })).await.unwrap();

        // List all tasks
        let resp = conn.send_request(METHOD_LIST, serde_json::json!({
            "limit": 100,
        })).await.unwrap();

        let tasks = resp.result.as_array().unwrap();
        assert!(tasks.len() >= 2, "expected at least 2 tasks, got {}", tasks.len());
    }

    #[tokio::test]
    async fn e2e_tail() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        let run_resp = conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "printf 'line1\\nline2\\nline3\\n'",
            "mode": "sync",
        })).await.unwrap();

        let task_id = run_resp.result["task_id"].as_str().unwrap();

        let tail_resp = conn.send_request(METHOD_TAIL, serde_json::json!({
            "task_id": task_id,
            "lines": 10,
        })).await.unwrap();

        let lines = tail_resp.result["lines"].as_array().unwrap();
        let line_strs: Vec<&str> = lines.iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(line_strs.contains(&"line1"));
        assert!(line_strs.contains(&"line2"));
        assert!(line_strs.contains(&"line3"));
    }

    #[tokio::test]
    async fn e2e_status() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        let resp = conn.send_request(METHOD_STATUS, serde_json::json!({})).await.unwrap();

        assert!(resp.result["uptime_secs"].as_u64().is_some());
        assert!(resp.result["tasks_running"].as_u64().is_some());
        assert!(resp.result["tasks_total"].as_u64().is_some());
    }

    #[tokio::test]
    async fn e2e_prune() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        // Create some tasks
        for i in 0..3 {
            conn.send_request(METHOD_RUN, serde_json::json!({
                "command": format!("echo prune_{}", i),
                "mode": "sync",
            })).await.unwrap();
        }

        // Prune keeping only 1
        let resp = conn.send_request(METHOD_PRUNE, serde_json::json!({
            "keep": 1,
        })).await.unwrap();

        assert!(resp.result["tasks_deleted"].as_u64().unwrap() >= 1);

        // Verify only 1 remains
        let list_resp = conn.send_request(METHOD_LIST, serde_json::json!({
            "limit": 100,
        })).await.unwrap();
        let tasks = list_resp.result.as_array().unwrap();
        assert_eq!(tasks.len(), 1);
    }

    // ── Task 2: Notification flow tests ────────────────────────────────────

    #[tokio::test]
    async fn e2e_notification_on_run() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        // Run a task async (so we can receive notifications while it runs)
        conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "echo notif_test",
            "mode": "async",
        })).await.unwrap();

        // Collect notifications for up to 5 seconds
        let mut got_update = false;
        let mut got_complete = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

        while tokio::time::Instant::now() < deadline {
            match conn.recv_notification_timeout(std::time::Duration::from_millis(200)).await {
                Some(notif) => {
                    match notif.method.as_str() {
                        "task/update" => {
                            got_update = true;
                            assert!(!notif.params["task_id"].as_str().unwrap().is_empty());
                        }
                        "task/complete" => {
                            got_complete = true;
                            assert_eq!(notif.params["exit_code"], 0);
                        }
                        "diagnostic" => {
                            // Parser may emit diagnostic events
                        }
                        _ => {}
                    }
                }
                None => continue,
            }
            if got_update && got_complete { break; }
        }

        assert!(got_update, "never received task/update notification");
        assert!(got_complete, "never received task/complete notification");
    }

    #[tokio::test]
    async fn e2e_notification_diagnostic() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        // Run a task that produces structured output (cc-like error)
        conn.send_request(METHOD_RUN, serde_json::json!({
            "command": "echo 'src/main.rs:10:5: error: undefined variable'",
            "mode": "async",
        })).await.unwrap();

        let mut got_diagnostic = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);

        while tokio::time::Instant::now() < deadline {
            if let Some(notif) = conn.recv_notification_timeout(std::time::Duration::from_millis(200)).await {
                if notif.method == "diagnostic" {
                    got_diagnostic = true;
                    let event = &notif.params["event"];
                    // The cc parser should match this
                    if event["type"].as_str() == Some("diagnostic") {
                        assert_eq!(event["severity"].as_str(), Some("error"));
                        break;
                    }
                }
            }
        }

        // At minimum we should get a raw diagnostic event
        assert!(got_diagnostic, "never received diagnostic notification");
    }

    // ── Task 3: Error path tests ───────────────────────────────────────────

    #[tokio::test]
    async fn e2e_unknown_method() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        let resp = conn.send_request("nonexistent/method", serde_json::json!({})).await.unwrap();

        // Error responses are wrapped in the result field
        assert!(resp.result.get("error").is_some(),
            "expected error response, got: {:?}", resp.result);
    }

    #[tokio::test]
    async fn e2e_kill_nonexistent_task() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        // Kill a task that doesn't exist — should not panic
        let resp = conn.send_request(METHOD_KILL, serde_json::json!({
            "task_id": "nonexistent-task-id",
        })).await.unwrap();

        // Should return success (idempotent kill)
        assert_eq!(resp.result["status"], "killed");
    }

    #[tokio::test]
    async fn e2e_query_missing_task_id() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        // Query without task_id — should return error
        let resp = conn.send_request(METHOD_QUERY, serde_json::json!({
            "limit": 10,
        })).await.unwrap();

        assert!(resp.result.get("error").is_some(),
            "expected error for missing task_id, got: {:?}", resp.result);
    }

    // ── Task 4: Concurrency and lifecycle tests ────────────────────────────

    #[tokio::test]
    async fn e2e_serial_runs_different_ids() {
        let (mut conn, _store, _tmp) = spawn_daemon_pair().await;

        let mut task_ids = Vec::new();
        for i in 0..5 {
            let resp = conn.send_request(METHOD_RUN, serde_json::json!({
                "command": format!("echo task_{}", i),
                "mode": "sync",
            })).await.unwrap();
            let task_id = resp.result["task_id"].as_str().unwrap().to_string();
            task_ids.push(task_id);
        }

        // All task IDs should be unique
        let unique: std::collections::HashSet<_> = task_ids.iter().collect();
        assert_eq!(unique.len(), 5, "expected 5 unique task IDs");
    }

    #[tokio::test]
    async fn e2e_parallel_connections() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Arc::new(Store::open(&db_path, false).unwrap());
        store.initialize_schema().unwrap();
        let parser = Arc::new(Engine::new(&ParserConfig::default()).unwrap());
        let bus = EventBus::new();
        let executor = Arc::new(Executor::new(store.clone(), parser, bus.clone()));

        // Create two separate connections
        let (c1, d1) = UnixStream::pair().unwrap();
        let (c2, d2) = UnixStream::pair().unwrap();

        let exec1 = executor.clone();
        let store1 = store.clone();
        let bus1 = bus.clone();
        tokio::spawn(async move {
            let _ = handle(d1, 0, exec1, store1, bus1).await;
        });

        let exec2 = executor.clone();
        let store2 = store.clone();
        let bus2 = bus.clone();
        tokio::spawn(async move {
            let _ = handle(d2, 1, exec2, store2, bus2).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let mut conn1 = DaemonConnection::new(c1);
        let mut conn2 = DaemonConnection::new(c2);

        // Both connections send requests in parallel
        let (r1, r2) = tokio::join!(
            conn1.send_request(METHOD_RUN, serde_json::json!({
                "command": "echo from_conn1", "mode": "sync",
            })),
            conn2.send_request(METHOD_RUN, serde_json::json!({
                "command": "echo from_conn2", "mode": "sync",
            })),
        );

        let resp1 = r1.unwrap();
        let resp2 = r2.unwrap();

        assert_eq!(resp1.result["status"], "completed");
        assert_eq!(resp2.result["status"], "completed");
        assert_ne!(
            resp1.result["task_id"].as_str().unwrap(),
            resp2.result["task_id"].as_str().unwrap(),
        );
    }

    #[tokio::test]
    async fn e2e_client_disconnect_no_panic() {
        let (client_stream, daemon_stream) = UnixStream::pair().unwrap();
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = Arc::new(Store::open(&db_path, false).unwrap());
        store.initialize_schema().unwrap();
        let parser = Arc::new(Engine::new(&ParserConfig::default()).unwrap());
        let bus = EventBus::new();
        let executor = Arc::new(Executor::new(store.clone(), parser, bus.clone()));

        let handle_task = tokio::spawn(async move {
            handle(daemon_stream, 0, executor, store, bus).await
        });

        // Drop the client immediately
        drop(client_stream);

        // Daemon handle should exit cleanly (not panic)
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            handle_task,
        ).await;

        assert!(result.is_ok(), "daemon handle task timed out");
        // The handle should return Ok (clean EOF)
    }
}
