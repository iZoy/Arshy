//! MCP initialization instructions and tool definitions.

use super::protocol::ToolDefinition;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "Route shell commands through arshy_exec(command:\"<cmd>\") instead of raw Bash; fall back \
     only when arshy is unreachable. mode:\"auto\" returns inspection output directly and parser-backed \
     commands as structured diagnostics. Pass cwd explicitly when needed. Use arshy_query for persisted \
     diagnostics and arshy_task only for cancellation, task listing, or original output."
        .into()
}

/// Return the complete list of MCP tool definitions.
///
/// Three single-purpose tools: execute, query diagnostics, and low-frequency
/// task lifecycle operations. Tool count is less important than avoiding one
/// conditional union schema whose fields change meaning by action.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "arshy_exec".into(),
            description: "Execute one shell command. mode:\"auto\" (default) returns read-only \
                         inspection output directly and parser-backed commands as structured \
                         diagnostics with file, line, and code."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type":"string","description":"Shell command to execute"},
                    "cwd": {"type":"string","description":"Absolute working directory for this command"},
                    "timeout_ms": {"type":"integer","description":"Timeout in ms"},
                    "mode": {"type":"string","enum":["auto","async"],"default":"auto",
                             "description":"auto: choose the lowest-overhead safe path; async: return a task id immediately"},
                    "env": {"type":"object","description":"Environment variables as key-value pairs"}
                },
                "required": ["command"]
            }),
            output_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "status": {"type": "string"},
                    "exit_code": {"type": ["integer", "null"]},
                    "duration_ms": {"type": ["integer", "null"]},
                    "primary_diagnostic": {"type": ["object", "null"]},
                    "raw_output": {"type": ["string", "null"]}
                },
                "required": ["task_id", "status", "exit_code"]
            })),
        },
        ToolDefinition {
            name: "arshy_query".into(),
            description: "Search persisted structured diagnostic events. Pass task_id for one \
                         structured task or omit it to search history. Raw log-only lines are \
                         excluded; use arshy_task(action:\"raw\") for original output."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string","description":"Persisted structured task ID; omit to search all task history"},
                    "event_type": {"type":"string","description":"Filter by emitted event type, such as diagnostic, test_result, summary, crash, data, or log"},
                    "severity": {"type":"string","enum":["error","warning","info"],"description":"Filter by severity"},
                    "code": {"type":"string","description":"Filter by error code"},
                    "file": {"type":"string","description":"Filter by file path"},
                    "limit": {"type":"integer","default":20,"description":"Max events to return"},
                    "offset": {"type":"integer","default":0,"description":"Pagination offset"}
                },
                "required": []
            }),
            output_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "events": {"type": "array", "items": {"type": "object"}},
                    "total": {"type": "integer"},
                    "limit": {"type": "integer"},
                    "offset": {"type": "integer"}
                },
                "required": ["events", "total", "limit", "offset"]
            })),
        },
        ToolDefinition {
            name: "arshy_task".into(),
            description: "Low-frequency task lifecycle operations: cancel a running task, list \
                         persisted tasks, or retrieve a task's original output."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {"type":"string","enum":["cancel","list","raw"],
                              "description":"Operation to perform"},
                    "task_id": {"type":"string","description":"Required for cancel and raw"},
                    "lines": {"type":"integer","default":200,
                              "description":"Original-output lines for raw; 0 returns all"},
                    "status": {"type":"string","enum":["running","completed","failed","timeout","killed"],
                              "description":"Optional list filter"},
                    "limit": {"type":"integer","default":10,"description":"Maximum tasks for list"}
                },
                "required": ["action"]
            }),
            output_schema: Some(serde_json::json!({
                "type": "object",
                "additionalProperties": true
            })),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_instructions_non_empty_and_names_tool() {
        let inst = default_instructions();
        assert!(!inst.is_empty());
        assert!(inst.contains("arshy_exec"));
    }

    /// The agent-visible MCP surface (instructions + tool descriptions +
    /// action enum) must stay lean: every token here is paid on every
    /// handshake. ~4 chars/token; budget allows ~20% headroom over the
    /// current trimmed size (~1350 chars ≈ 340 tokens).
    #[test]
    fn mcp_surface_stays_lean() {
        let inst = default_instructions();
        let tools = tool_definitions();
        let exec = tools.iter().find(|t| t.name == "arshy_exec").unwrap();
        let query = tools.iter().find(|t| t.name == "arshy_query").unwrap();
        let task = tools.iter().find(|t| t.name == "arshy_task").unwrap();
        let action_desc = task
            .input_schema
            .get("properties")
            .and_then(|p| p.get("action"))
            .and_then(|a| a.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let total_chars = inst.len()
            + exec.description.len()
            + query.description.len()
            + task.description.len()
            + action_desc.len();

        assert!(
            total_chars <= 1600,
            "MCP surface grew to {total_chars} chars (~{} tokens) — trim before merging",
            total_chars / 4
        );

        assert!(!exec.input_schema["properties"].as_object().unwrap().contains_key("action"));
        let properties = exec.input_schema["properties"].as_object().unwrap();
        assert!(!properties.contains_key("parse_hint"));
        assert_eq!(properties["mode"]["enum"], serde_json::json!(["auto", "async"]));
    }

    #[test]
    fn tool_definitions_have_expected_tools() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"arshy_exec"));
        assert!(names.contains(&"arshy_query"));
        assert!(names.contains(&"arshy_task"));
        for t in &tools {
            assert!(!t.description.is_empty(), "tool {} has empty description", t.name);
            assert_eq!(
                t.input_schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "tool {} input_schema must be an object",
                t.name
            );
        }
    }
}
