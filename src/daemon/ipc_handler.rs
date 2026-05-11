//! Daemon IPC handler — accepts UDS connections and dispatches JSON-RPC requests.

use arshy_lib::ipc::{
    self, ErrorResponse, JsonRpcError, QueryParams, Request, Response, RunTaskParams,
    METHOD_KILL, METHOD_LIST, METHOD_PRUNE, METHOD_QUERY, METHOD_RUN, METHOD_STATUS, METHOD_TAIL,
};
use arshy_lib::Result;
use serde::Serialize;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader, BufWriter};
use tokio::net::UnixStream;

use super::bus::EventBus;
use super::exec::Executor;
use super::store::Store;

/// Result returned from the executor after scheduling a task.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub task_id: String,
    pub status: arshy_lib::ipc::TaskStatus,
    pub pid: Option<u32>,
}

/// Handle a single UDS connection — read JSON-RPC requests, dispatch, write responses.
pub async fn handle(
    stream: UnixStream,
    _conn_id: u64,
    executor: Arc<Executor>,
    store: Arc<Store>,
    _event_bus: EventBus,
) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);
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
                ipc::write_json_line(&mut writer, &err).await?;
                continue;
            }
        };

        match dispatch(&request, &executor, &store).await {
            Ok(result) => {
                let response = Response { jsonrpc: "2.0".into(), id: request.id, result };
                ipc::write_json_line(&mut writer, &response).await?;
            }
            Err(e) => {
                let err = ErrorResponse {
                    jsonrpc: "2.0".into(),
                    id: request.id,
                    error: JsonRpcError { code: -32603, message: e.to_string() },
                };
                ipc::write_json_line(&mut writer, &err).await?;
            }
        }
    }

    Ok(())
}

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
                .params
                .get("status")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit: usize = request
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10) as usize;
            let tasks = store.list_tasks(status.as_deref(), limit)?;
            Ok(serde_json::to_value(&tasks)?)
        }
        METHOD_KILL => {
            let task_id = request
                .params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| arshy_lib::ArshyError::Ipc("missing task_id".into()))?;
            executor.kill(task_id).await?;
            Ok(serde_json::json!({ "task_id": task_id, "status": "killed" }))
        }
        METHOD_TAIL => {
            let task_id = request
                .params
                .get("task_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| arshy_lib::ArshyError::Ipc("missing task_id".into()))?;
            let lines: usize = request
                .params
                .get("lines")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;
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
            let task = store.list_tasks(None, 0)?;
            let running = store
                .list_tasks(Some("running"), 0)?
                .len();
            Ok(serde_json::json!({
                "uptime_secs": daemon_uptime_secs(),
                "tasks_running": running,
                "tasks_total": task.len(),
            }))
        }
        METHOD_PRUNE => {
            let keep: Option<usize> = request
                .params
                .get("keep")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let older_than: Option<u32> = request
                .params
                .get("older_than")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);

            let (tasks_deleted, events_deleted) = if let Some(k) = keep {
                store.prune_keep(k)?
            } else if let Some(days) = older_than {
                store.prune_older_than(days)?
            } else {
                store.prune_keep(1000)?
            };

            Ok(serde_json::json!({
                "tasks_deleted": tasks_deleted,
                "events_deleted": events_deleted,
            }))
        }
        _ => Err(arshy_lib::ArshyError::Ipc(format!(
            "unknown method: {}",
            request.method
        ))),
    }
}

fn daemon_uptime_secs() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let t = START.get_or_init(std::time::Instant::now);
    t.elapsed().as_secs()
}
