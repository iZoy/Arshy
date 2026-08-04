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
pub const METHOD_STATUS: &str = "daemon/status";
pub const METHOD_PRUNE: &str = "daemon/prune";
pub const METHOD_SHUTDOWN: &str = "daemon/shutdown";
pub const METHOD_STATS: &str = "daemon/stats";
pub const METHOD_HEALTH: &str = "daemon/health";
pub const METHOD_CD: &str = "session/cd";
pub const METHOD_SUBSCRIBE: &str = "task/subscribe";
pub const METHOD_PARSER_RELOAD: &str = "parser/reload";
pub const METHOD_ANALYZE: &str = "daemon/analyze";

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
    /// Purpose label (e.g. `dogfood`, `sample`). `None`/`real` = real
    /// development workload; stats split these out of failure metrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
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
    /// Purpose label for stats (e.g. `dogfood`). Untagged tasks count as real.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
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
    pub task_id: Option<String>,
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
    /// Include raw log events in results. Default false — only structured
    /// events (diagnostic, crash, test_result, summary, etc.) are returned.
    /// Log events are unstructured lines that add bulk without helping agents.
    #[serde(default)]
    pub include_logs: bool,
}

fn default_limit() -> usize {
    20
}

impl Default for QueryParams {
    fn default() -> Self {
        Self {
            task_id: None,
            event_type: None,
            severity: None,
            code: None,
            file: None,
            limit: default_limit(),
            offset: 0,
            include_logs: false,
        }
    }
}

// ── Stats response ──────────────────────────────────────────────────────────

/// Per-purpose stats slice — lets stats separate test workloads (dogfood
/// fixtures, sample-project runs) from real development failure metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurposeStats {
    /// `real` (untagged tasks), `dogfood`, `sample`, ...
    pub purpose: String,
    pub total: u64,
    pub failed: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsResponse {
    pub total_tasks: u64,
    pub by_status: StatusCounts,
    pub total_events: u64,
    pub total_errors: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub purpose_breakdown: Vec<PurposeStats>,
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
    /// Number of error/warning events enriched with source context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_enriched: Option<u64>,
    /// Total duplicate events collapsed by deduplicator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedup_collapsed: Option<u64>,
    /// Total errors correlated with recent git changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlated_errors: Option<u64>,
    /// Per-parser usage counts (top parsers by task count).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_parser_usage: Option<Vec<ParserCount>>,
    /// Total raw output bytes across all tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_raw_output_bytes: Option<u64>,
    /// Total structured events bytes across all tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_agent_delivered_bytes: Option<u64>,
    /// Total agent-visible events (non-log) across all tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_agent_visible_events: Option<u64>,
    /// Total agent-skipped events (log type) across all tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_agent_skipped_events: Option<u64>,
    /// Total locations extracted across all tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_locations_extracted: Option<u64>,
    /// Total error codes extracted across all tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_codes_extracted: Option<u64>,
    /// Total contexts enriched across all tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_contexts_enriched: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParserCount {
    pub parser: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusCounts {
    pub running: u64,
    pub completed: u64,
    pub failed: u64,
    pub killed: u64,
    pub timeout: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_terminal_classifies_statuses() {
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Killed.is_terminal());
        assert!(TaskStatus::Timeout.is_terminal());
    }

    #[test]
    fn method_constants_are_well_formed() {
        assert_eq!(METHOD_RUN, "task/run");
        assert_eq!(METHOD_QUERY, "task/query");
        assert_eq!(METHOD_LIST, "task/list");
        assert_eq!(METHOD_KILL, "task/kill");
        assert_eq!(METHOD_TAIL, "task/tail");
        assert_eq!(METHOD_STATUS, "daemon/status");
        assert_eq!(METHOD_PRUNE, "daemon/prune");
        assert_eq!(METHOD_SHUTDOWN, "daemon/shutdown");
        assert_eq!(METHOD_STATS, "daemon/stats");
        assert_eq!(METHOD_HEALTH, "daemon/health");
        assert_eq!(METHOD_CD, "session/cd");
        assert_eq!(METHOD_SUBSCRIBE, "task/subscribe");
        assert_eq!(METHOD_PARSER_RELOAD, "parser/reload");
        assert_eq!(METHOD_ANALYZE, "daemon/analyze");
    }

    #[test]
    fn method_constants_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for m in [
            METHOD_RUN,
            METHOD_QUERY,
            METHOD_LIST,
            METHOD_KILL,
            METHOD_TAIL,
            METHOD_STATUS,
            METHOD_PRUNE,
            METHOD_SHUTDOWN,
            METHOD_STATS,
            METHOD_HEALTH,
            METHOD_CD,
            METHOD_SUBSCRIBE,
            METHOD_PARSER_RELOAD,
            METHOD_ANALYZE,
        ] {
            assert!(seen.insert(m), "duplicate method constant: {m}");
        }
    }

    #[test]
    fn error_codes_are_negative_and_distinct() {
        let codes = [
            error_code::PARSE_ERROR,
            error_code::INVALID_REQUEST,
            error_code::METHOD_NOT_FOUND,
            error_code::INVALID_PARAMS,
            error_code::INTERNAL_ERROR,
            error_code::TASK_NOT_FOUND,
            error_code::TASK_TIMEOUT,
            error_code::ACCESS_DENIED,
            error_code::COMMAND_BLOCKED,
            error_code::RATE_LIMITED,
        ];
        assert!(codes.iter().all(|c| *c < 0), "error codes must be negative");
        let mut seen = std::collections::HashSet::new();
        for c in codes {
            assert!(seen.insert(c), "duplicate error code: {c}");
        }
    }
}
