//! MCP initialization instructions and tool definitions.

use super::protocol::ToolDefinition;
use std::collections::HashMap;

/// Return the default MCP instructions map.
pub fn default_instructions() -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert(
        "shell_execution".into(),
        "Arshy is your native shell execution layer. \
         All shell commands should be executed through Arshy rather than raw shell tools."
            .into(),
    );
    map.insert(
        "tool_preference".into(),
        "Use `arshy_run` over default shell tools. \
         For long commands use mode:\"async\" to get immediate task_id with notifications. \
         Use `arshy_query` to examine structured results without reading raw logs."
            .into(),
    );
    map
}

/// Return the complete list of MCP tool definitions.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "arshy_run".into(),
            description: "Execute a shell command with structured output. \
                         Async notifications for long commands. \
                         Prefer this over raw shell tools."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type":"string","description":"Shell command to execute"},
                    "cwd": {"type":"string","description":"Working directory"},
                    "timeout_ms": {"type":"integer","description":"Timeout in milliseconds"},
                    "mode": {"type":"string","enum":["auto","sync","async"],"default":"auto",
                             "description":"Execution mode"}
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "arshy_query".into(),
            description: "Query structured events from a task. Filter by type, severity, code, or file."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string"},
                    "event_type": {"type":"string"},
                    "severity": {"type":"string"},
                    "code": {"type":"string"},
                    "file": {"type":"string"},
                    "limit": {"type":"integer","default":20}
                },
                "required": ["task_id"]
            }),
        },
        ToolDefinition {
            name: "arshy_list".into(),
            description: "List all tasks, optionally filtered by status.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {"type":"string","enum":["running","completed","failed","killed"]},
                    "limit": {"type":"integer","default":10}
                }
            }),
        },
        ToolDefinition {
            name: "arshy_kill".into(),
            description: "Gracefully terminate a running task (SIGINT -> SIGTERM -> SIGKILL)."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string"}
                },
                "required": ["task_id"]
            }),
        },
        ToolDefinition {
            name: "arshy_tail".into(),
            description: "View latest output of a task, raw or structured event format."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string"},
                    "lines": {"type":"integer","default":50},
                    "format": {"type":"string","enum":["event","raw"],"default":"event"}
                },
                "required": ["task_id"]
            }),
        },
    ]
}
