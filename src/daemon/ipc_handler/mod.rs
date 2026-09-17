//! Daemon IPC handler — bidirectional JSON-RPC over UDS.
//!
//! Architecture:
//! - Reader task: reads requests from proxy, dispatches, sends responses via channel
//! - Writer task: drains channel (responses + notifications) to the socket
//! - EventBus notifications are forwarded to all connected proxies

use crate::ipc::{
    self, ErrorResponse, JsonRpcError, Notification, QueryParams, Request, Response, RunTaskParams,
    METHOD_CD, METHOD_HEALTH, METHOD_KILL, METHOD_LIST, METHOD_PRUNE, METHOD_QUERY, METHOD_RUN,
    METHOD_SHUTDOWN, METHOD_STATS, METHOD_STATUS, METHOD_SUBSCRIBE, METHOD_TAIL,
};
use crate::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch};

use super::bus::{BusEvent, BusEventKind, EventBus};
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
    pub status: crate::ipc::TaskStatus,
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_count: Option<u64>,
    /// Number of visible structured events available through `arshy_query`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<String>,
    #[serde(default)]
    pub short_command: bool,
    /// A representative error-level diagnostic event (if any).
    /// This is evidence selected for the agent, not a claim about causality.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_diagnostic: Option<serde_json::Value>,
    /// Project context: recent git changes, related files, etc.
    /// Helps agent understand what changed before the command ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_context: Option<serde_json::Value>,
    /// Size of the raw PTY output in bytes (only meaningful for long commands;
    /// short commands return raw_output verbatim). Used by metrics tooling
    /// for diagnostics and offline analytics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_output_bytes: Option<u64>,
}

// ── Main handler entry ──────────────────────────────────────────────────────

pub async fn handle(
    stream: UnixStream,
    conn_id: u64,
    executor: Arc<Executor>,
    store: Arc<Store>,
    event_bus: EventBus,
    shutdown_tx: watch::Sender<bool>,
) -> Result<()> {
    let (reader_half, writer_half) = stream.into_split();
    let mut reader = BufReader::new(reader_half);
    let mut writer = BufWriter::new(writer_half);

    let (tx, rx) = mpsc::channel::<Outbound>(1024);

    let writer_handle = tokio::spawn(async move {
        if let Err(e) = drain_outbound(&mut writer, rx).await {
            tracing::error!("conn {} writer error: {}", conn_id, e);
        }
    });

    let mut bus_rx = event_bus.subscribe();
    let tx_notif = tx.clone();
    let notif_handle = tokio::spawn(async move {
        loop {
            let event = match bus_rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        "connection {} skipped {} lagged notifications",
                        conn_id,
                        skipped
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if let Some(notification) = bus_event_to_notification(&event) {
                match tx_notif.try_send(Outbound::Notification(notification)) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        tracing::debug!(
                            "connection {} notification dropped: outbound queue full",
                            conn_id
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
        }
    });

    let mut line = String::new();
    // Initialize the default cwd to the daemon's startup directory. Without
    // this, a run() call from an agent that never invoked session/cd would
    // carry cwd=None, which causes compute_enhanced_project_context to leak
    // the daemon's git diff stat into an unrelated command result.
    let mut default_cwd: Option<String> =
        std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned());
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
                    error: JsonRpcError {
                        code: ipc::error_code::PARSE_ERROR,
                        message: e.to_string(),
                        data: None,
                    },
                };
                let _ = tx
                    .send(Outbound::Response(Response {
                        jsonrpc: "2.0".into(),
                        id: 0,
                        result: serde_json::to_value(&err).unwrap_or_default(),
                    }))
                    .await;
                continue;
            }
        };

        // Intercept METHOD_RUN to inject default_cwd, handle METHOD_CD locally
        let response = if request.method.as_str() == METHOD_RUN {
            let mut req = request.clone();
            if req.params.get("cwd").is_none() {
                if let Some(ref cwd) = default_cwd {
                    req.params["cwd"] = serde_json::json!(cwd);
                }
            }
            dispatch(&req, &executor, store.clone(), &shutdown_tx).await
        } else if request.method.as_str() == METHOD_CD {
            let dir =
                request.params.get("command").and_then(|v| v.as_str()).unwrap_or(".").to_string();
            let abs = tokio::task::spawn_blocking(move || {
                std::fs::canonicalize(std::path::Path::new(&dir))
                    .map_err(|e| crate::ArshyError::Ipc(format!("cd: {}: {}", dir, e)))
            })
            .await
            .map_err(|e| crate::ArshyError::Ipc(format!("cd resolve panicked: {}", e)))??;
            let cwd_str = abs.to_string_lossy().to_string();
            default_cwd = Some(cwd_str.clone());
            Ok(serde_json::json!({"cwd": cwd_str}))
        } else if request.method.as_str() == METHOD_SUBSCRIBE {
            let tid_val = request.params.get("task_id").and_then(|v| v.as_str());
            match tid_val {
                None => Err(crate::ArshyError::Ipc("missing task_id".into())),
                Some(task_id_str) => {
                    validate_task_id(task_id_str)?;
                    let task_id = task_id_str.to_string();

                    // Subscribe before checking state. If completion happens
                    // between these two operations, it is then visible either
                    // in the store or in this receiver; the previous reverse
                    // order could miss the broadcast and wait until timeout.
                    let mut bus_rx = event_bus.subscribe();
                    let immediate =
                        store.get_task(&task_id).ok().flatten().filter(|t| t.status.is_terminal());

                    if let Some(task) = immediate {
                        Ok(serde_json::json!({
                            "task_id": task.task_id,
                            "status": task.status,
                            "exit_code": task.exit_code,
                            "duration_ms": task.duration_ms,
                        }))
                    } else {
                        let timeout = tokio::time::Duration::from_secs(600);
                        loop {
                            tokio::select! {
                                event = bus_rx.recv() => {
                                    match event {
                                        Ok(BusEvent {
                                            kind: BusEventKind::TaskComplete {
                                                task_id: ref tid, ref status, exit_code, duration_ms
                                            }, ..
                                        }) if tid == &task_id => {
                                            break Ok(serde_json::json!({
                                                "task_id": tid,
                                                "exit_code": exit_code,
                                                "duration_ms": duration_ms,
                                                "status": status,
                                            }));
                                        }
                                        Ok(_) => continue,
                                        Err(_) => break Err(crate::ArshyError::Ipc(
                                            "event bus disconnected".into()
                                        )),
                                    }
                                }
                                _ = tokio::time::sleep(timeout) => {
                                    break Err(crate::ArshyError::Ipc(format!(
                                        "subscribe {} timed out after 600s", task_id
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        } else {
            dispatch(&request, &executor, store.clone(), &shutdown_tx).await
        };
        let outbound = match response {
            Ok(result) => {
                Outbound::Response(Response { jsonrpc: "2.0".into(), id: request.id, result })
            }
            Err(e) => {
                // Build error context from request params
                let mut data = serde_json::json!({ "retryable": e.is_retryable() });
                if let Some(cmd) = request.params.get("command").and_then(|v| v.as_str()) {
                    data["command"] = serde_json::json!(cmd);
                }
                if let Some(tid) = request.params.get("task_id").and_then(|v| v.as_str()) {
                    data["task_id"] = serde_json::json!(tid);
                }
                Outbound::Response(Response {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    result: serde_json::to_value(&ErrorResponse {
                        jsonrpc: "2.0".into(),
                        id: request.id,
                        error: JsonRpcError {
                            code: e.json_rpc_code(),
                            message: e.to_string(),
                            data: Some(data),
                        },
                    })
                    .unwrap_or_default(),
                })
            }
        };

        if tx.send(outbound).await.is_err() {
            break;
        }
    }

    drop(tx);
    notif_handle.abort();
    let _ = writer_handle.await;

    tracing::debug!("conn {} closed", conn_id);
    Ok(())
}

// ── Request dispatch ────────────────────────────────────────────────────────

async fn dispatch(
    request: &Request,
    executor: &Executor,
    store: Arc<Store>,
    shutdown_tx: &watch::Sender<bool>,
) -> Result<serde_json::Value> {
    match request.method.as_str() {
        METHOD_RUN | METHOD_KILL if executor.access_level() == "read-only" => {
            Err(crate::ArshyError::AccessDenied("read-only mode".into()))
        }
        METHOD_RUN => {
            let params: RunTaskParams = serde_json::from_value(request.params.clone())?;
            let result = executor
                .run(
                    &params.command,
                    params.cwd.as_deref(),
                    params.timeout_ms,
                    &params.mode,
                    params.parse_hint.as_deref(),
                    params.env.as_ref(),
                    params.errors_only,
                    params.purpose.as_deref(),
                    params.dedup_key.as_deref(),
                )
                .await?;
            let mut resp = serde_json::to_value(&result)?;
            // Adaptive inline events: keep the one-round-trip flow (validated by
            // dogfooding — see commit ec523d3) while honoring token restraint.
            // Failure → up to 20 error-severity events; success → up to 5
            // warning/info events. Full detail stays available via arshy_query.
            let failed = result.exit_code.unwrap_or(0) != 0;
            let (inline_limit, severity_filter) =
                if failed { (20, Some("error".to_string())) } else { (5, None) };
            let query = QueryParams {
                task_id: Some(result.task_id.clone()),
                limit: inline_limit,
                severity: severity_filter,
                include_logs: false,
                ..Default::default()
            };
            if let Ok((events, total)) = store.query_events(&query) {
                let shown = events.len();
                // On failures the inline sample is error-only, but
                // event_count must still describe every visible structured
                // event (including warnings/info) available to arshy_query.
                let visible_total = result.event_count.unwrap_or(total as u64) as usize;
                resp["events"] = serde_json::to_value(&events).unwrap_or_default();
                resp["event_count"] = serde_json::json!(visible_total);
                if let Some((truncated, hint)) =
                    truncation_hint(shown, visible_total, &result.task_id)
                {
                    resp["events_truncated"] = serde_json::json!(truncated);
                    resp["events_hint"] = serde_json::json!(hint);
                }
            }
            Ok(resp)
        }
        METHOD_QUERY => {
            let mut params: QueryParams = serde_json::from_value(request.params.clone())?;
            if let Some(task_id) = params.task_id.as_deref() {
                validate_task_id(task_id)?;
            }
            params.limit = params.limit.min(1000);
            // task_id present → per-task query (unchanged shape); absent →
            // cross-task search across all tasks, events carry their task_id.
            let (events, total) = match &params.task_id {
                Some(_) => {
                    let (evts, total) = store.query_events(&params)?;
                    let evts = evts
                        .into_iter()
                        .map(|e| {
                            serde_json::to_value(&e)
                                .map_err(|err| crate::ArshyError::Ipc(err.to_string()))
                        })
                        .collect::<std::result::Result<Vec<serde_json::Value>, _>>()?;
                    (evts, total)
                }
                None => store.search_events(&params)?,
            };
            // Restricted code references (non-obvious exit/error codes) are
            // attached here, on demand — never inlined into stored events.
            let fallback_tool = match &params.task_id {
                Some(tid) => store.get_task(tid).ok().flatten().and_then(|t| t.parser_name),
                None => None,
            };
            let events = attach_references(events, executor, &store, fallback_tool);
            Ok(serde_json::json!({ "events": events, "total": total }))
        }
        METHOD_LIST => {
            let status: Option<String> =
                request.params.get("status").and_then(|v| v.as_str()).map(String::from);
            let limit: usize =
                request.params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10).min(1000)
                    as usize;
            let tasks = store.list_tasks(status.as_deref(), limit)?;
            Ok(serde_json::to_value(&tasks)?)
        }
        METHOD_KILL => {
            let task_id = request
                .params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| crate::ArshyError::Ipc("missing task_id".into()))?;
            validate_task_id(task_id)?;
            executor.kill(task_id).await?;
            Ok(serde_json::json!({ "task_id": task_id, "status": "cancelling" }))
        }
        METHOD_TAIL => {
            let task_id = request
                .params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| crate::ArshyError::Ipc("missing task_id".into()))?;
            validate_task_id(task_id)?;
            let lines: usize =
                request.params.get("lines").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let format: String = request
                .params
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("event")
                .to_string();
            let output = executor.tail(task_id, lines, &format).await?;
            Ok(serde_json::json!({ "task_id": task_id, "lines": output }))
        }
        METHOD_STATUS => {
            let all = store.list_tasks(None, 10_000)?;
            let running = store.list_tasks(Some("running"), 10_000)?.len();
            let db_size = store.get_stats().ok().and_then(|s| s.db_size_bytes);
            Ok(serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_secs": daemon_uptime_secs(),
                "tasks_running": running,
                "tasks_total": all.len(),
                "db_size_bytes": db_size,
                "parser_count": executor.parser_count(),
                "counters": super::telemetry::snapshot(),
            }))
        }
        METHOD_HEALTH => {
            // Deep health check — verifies store and executor are functional.
            let store_ok = store.list_tasks(Some("running"), 1).is_ok();
            let running_count =
                store.list_tasks(Some("running"), 10_000).map(|t| t.len()).unwrap_or(0);
            let total_tasks = store.list_tasks(None, 1).map(|t| t.len()).unwrap_or(0);
            Ok(serde_json::json!({
                "status": if store_ok { "ok" } else { "degraded" },
                "store_ok": store_ok,
                "tasks_running": running_count,
                "tasks_total": total_tasks,
                "uptime_secs": daemon_uptime_secs(),
                "counters": super::telemetry::snapshot(),
            }))
        }
        METHOD_PRUNE => {
            let keep = request.params.get("keep").and_then(|v| v.as_u64()).map(|v| v as usize);
            let older_than =
                request.params.get("older_than").and_then(|v| v.as_u64()).map(|v| v as u32);
            let (td, ed) = if let Some(k) = keep {
                store.prune_keep(k)?
            } else if let Some(d) = older_than {
                store.prune_older_than(d)?
            } else {
                store.prune_keep(1000)?
            };
            Ok(serde_json::json!({ "tasks_deleted": td, "events_deleted": ed }))
        }
        METHOD_SHUTDOWN => {
            let _ = shutdown_tx.send(true);
            Ok(serde_json::json!({ "status": "shutting_down" }))
        }
        METHOD_STATS => {
            let stats = store.get_stats()?;
            Ok(serde_json::to_value(&stats)?)
        }
        ipc::METHOD_PARSER_RELOAD => {
            let diff = executor.reload_parsers()?;
            let reference_diff = executor.reload_reference()?;
            Ok(serde_json::json!({ "diff": diff, "reference": reference_diff }))
        }
        ipc::METHOD_ANALYZE => {
            let store_clone = store.clone();
            let report = tokio::task::spawn_blocking(move || {
                let analytics = super::analytics::Analytics::new(&store_clone);
                analytics.generate_report()
            })
            .await
            .map_err(|e| crate::ArshyError::Other(format!("analytics panic: {}", e)))??;
            Ok(serde_json::to_value(&report)?)
        }
        _ => Err(crate::ArshyError::Ipc(format!("unknown method: {}", request.method))),
    }
}

fn validate_task_id(task_id: &str) -> Result<()> {
    uuid::Uuid::parse_str(task_id)
        .map(|_| ())
        .map_err(|_| crate::ArshyError::Ipc("invalid task_id".into()))
}

// ── Writer task ─────────────────────────────────────────────────────────────

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

fn bus_event_to_notification(event: &BusEvent) -> Option<Notification> {
    use super::bus::router::NotificationRouter;
    NotificationRouter::to_notification(event)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn daemon_uptime_secs() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs()
}

/// Build the `events_truncated` / `events_hint` fields for a run response when
/// the inlined event sample is smaller than the total event count.
///
/// Returns `None` when everything fits (no truncation signal needed).
fn truncation_hint(shown: usize, total: usize, task_id: &str) -> Option<(bool, String)> {
    if shown < total {
        Some((
            true,
            format!(
                "Showing {}/{} events. Call arshy_query(task_id:\"{}\") for the rest.",
                shown, total, task_id
            ),
        ))
    } else {
        None
    }
}

/// Attach restricted reference data (what a non-obvious exit/error code
/// *means*) to query results. Lookup is scoped to the task's tool when
/// known (per-task query → task parser_name; cross-task search → each
/// event's task_id), so docker 130 and aws 130 never mix. Events without a
/// matching reference pass through unchanged.
fn attach_references(
    events: Vec<serde_json::Value>,
    executor: &Executor,
    store: &Store,
    fallback_tool: Option<String>,
) -> Vec<serde_json::Value> {
    // task_id → tool cache for cross-task search (bounded by event count).
    let mut tool_cache: HashMap<String, Option<String>> = HashMap::new();
    events
        .into_iter()
        .map(|ev| {
            let Some(code) = ev.get("code").and_then(|c| c.as_str()) else {
                return ev;
            };
            let tool = ev
                .get("task_id")
                .and_then(|t| t.as_str())
                .and_then(|tid| {
                    tool_cache.get(tid).cloned().unwrap_or_else(|| {
                        let t =
                            store.get_task(tid).ok().flatten().and_then(|task| task.parser_name);
                        tool_cache.insert(tid.to_string(), t.clone());
                        t
                    })
                })
                .or_else(|| fallback_tool.clone());
            let Some(entries) = executor.reference_entries(code, tool.as_deref()) else {
                return ev;
            };
            let mut obj = ev;
            obj["reference"] = serde_json::to_value(entries).unwrap_or_default();
            obj
        })
        .collect()
}

// ── Integration tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
