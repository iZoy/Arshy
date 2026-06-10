//! IPC protocol between `arshy` proxy and `arshyd` daemon.
//! JSON-RPC 2.0 over Unix Domain Socket, JSON Lines framing.

mod transport;

pub use transport::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Method names ─────────────────────────────────────────────────────────────

pub const METHOD_RUN: &str = "task/run";
pub const METHOD_QUERY: &str = "task/query";
pub const METHOD_LIST: &str = "task/list";
pub const METHOD_KILL: &str = "task/kill";
pub const METHOD_TAIL: &str = "task/tail";
pub const METHOD_STDIN: &str = "task/stdin";
pub const METHOD_STATUS: &str = "daemon/status";
pub const METHOD_PRUNE: &str = "daemon/prune";
pub const METHOD_SHUTDOWN: &str = "daemon/shutdown";
pub const METHOD_STATS: &str = "daemon/stats";
pub const METHOD_HEALTH: &str = "daemon/health";
pub const METHOD_CD: &str = "session/cd";
pub const METHOD_SUBSCRIBE: &str = "task/subscribe";
pub const METHOD_PARSER_RELOAD: &str = "parser/reload";

pub const NOTIF_TASK_UPDATE: &str = "task/update";
pub const NOTIF_TASK_COMPLETE: &str = "task/complete";
pub const NOTIF_DIAGNOSTIC: &str = "diagnostic";
pub const NOTIF_DAEMON_SHUTDOWN: &str = "daemon/shutdown";

// ── JSON-RPC error codes ────────────────────────────────────────────────────

/// Standard JSON-RPC 2.0 error codes.
pub mod error_code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;

    // Application-defined codes (outside -32000..-32099 reserved range)
    pub const TASK_NOT_FOUND: i64 = -32001;
    pub const TASK_TIMEOUT: i64 = -32002;
    pub const ACCESS_DENIED: i64 = -32003;
    pub const COMMAND_BLOCKED: i64 = -32004;
    pub const RATE_LIMITED: i64 = -32005;
}

// ── JSON-RPC types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    pub result: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub error: JsonRpcError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

// ── Task / Event data types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Completed,
    Failed,
    Killed,
    Timeout,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Killed | Self::Timeout)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub task_id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub status: TaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parser_name: Option<String>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub events_count: u64,
    pub error_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub seq: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<EventLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<EventContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<EventHint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventLocation {
    pub file: String,
    pub line: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventContext {
    #[serde(default)]
    pub before: Vec<String>,
    pub line: String,
    #[serde(default)]
    pub after: Vec<String>,
}

/// Suggested retry commands for when an error occurs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryHint {
    /// Suggested commands to retry (in order)
    pub commands: Vec<String>,
    /// Why this retry might help
    pub reason: String,
}

/// Fix suggestion attached to an error event via error code lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventHint {
    pub cause: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryHint>,
}

// ── Specific param / response types ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTaskParams {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Hint for expected output format: "json", "csv", "table", "raw".
    /// When set, the executor prioritizes the matching parser and bypasses
    /// the zero-overhead short path to ensure structured output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_hint: Option<String>,
    /// Environment variables for the command: {"KEY": "value", ...}.
    /// Inherited from the parent process; these entries add or override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    /// Only return error-severity events. Reduces output for large builds.
    #[serde(default)]
    pub errors_only: bool,
}

fn default_mode() -> String {
    "auto".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTaskResponse {
    pub task_id: String,
    pub status: TaskStatus,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParams {
    pub task_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    20
}

// ── Stats response ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsResponse {
    pub total_tasks: u64,
    pub by_status: StatusCounts,
    pub total_events: u64,
    pub total_errors: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p50_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p99_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_size_bytes: Option<u64>,
    /// Parser coverage: percentage of events that aren't raw log fallback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parser_coverage_pct: Option<f64>,
    /// Number of error events that received fix hints from HintDb.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints_attached: Option<u64>,
    /// Number of error/warning events enriched with source context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_enriched: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusCounts {
    pub running: u64,
    pub completed: u64,
    pub failed: u64,
    pub killed: u64,
    pub timeout: u64,
}
