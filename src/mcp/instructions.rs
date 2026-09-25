//! MCP initialization instructions and tool definitions.

use super::protocol::ToolDefinition;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "Use arshy_exec for shell commands. It runs non-interactive sh -c with null stdin and separate \
     stdout/stderr pipes (no PTY). Pass cwd when needed. mode=auto returns short inspection output \
     inline and parses configured tools; mode=async returns a task handle. Use arshy_query for saved \
     diagnostics and arshy_task for raw output, cancellation, or listing."
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
            description: "Run one non-interactive sh -c command (null stdin, separate stdout/stderr, no PTY). \
                         Pass cwd, timeout_ms, or env as needed. auto selects inline inspection or parsed output; \
                         async returns a task handle."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type":"string","description":"Command for sh -c"},
                    "cwd": {"type":"string","description":"Working directory"},
                    "timeout_ms": {"type":"integer","description":"Execution timeout"},
                    "mode": {"type":"string","enum":["auto","async"],"default":"auto",
                             "description":"auto: inline or parsed; async: return a task handle"},
                    "env": {"type":"object","description":"Environment overrides"}
                },
                "required": ["command"]
            }),
            output_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {"type": "string"},
                    "exit_code": {"type": ["integer", "null"]},
                    "diagnostic": {"type": "object", "description":"Failure facts when known",
                        "properties": {
                            "severity": {"type":"string"},
                            "code": {"type":"string"},
                            "location": {"type":"object"}
                        }, "additionalProperties": false},
                    "task_id": {"type": "string", "description":"Present only while running or when output is incomplete"}
                },
                "required": ["status", "exit_code"]
            })),
        },
        ToolDefinition {
            name: "arshy_query".into(),
            description: "Search saved diagnostics by task or across history. Use arshy_task raw for command output."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string","description":"Task ID; omit to search history"},
                    "event_type": {"type":"string","description":"Event type filter"},
                    "severity": {"type":"string","enum":["error","warning","info"],"description":"Severity filter"},
                    "code": {"type":"string","description":"Code filter"},
                    "file": {"type":"string","description":"Path filter"},
                    "limit": {"type":"integer","default":20,"description":"Page size"},
                    "offset": {"type":"integer","default":0,"description":"Page offset"}
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
            description: "Cancel a task, list tasks, or retrieve captured output."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {"type":"string","enum":["cancel","list","raw"],
                              "description":"Operation"},
                    "task_id": {"type":"string","description":"Task ID for cancel or raw"},
                    "lines": {"type":"integer","default":200,
                              "description":"Raw lines; 0 returns all"},
                    "status": {"type":"string","enum":["running","completed","failed","timeout","killed"],
                              "description":"List filter"},
                    "limit": {"type":"integer","default":10,"description":"List limit"}
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

    fn description_chars(value: &serde_json::Value) -> usize {
        match value {
            serde_json::Value::Object(object) => object
                .iter()
                .map(|(key, value)| {
                    if key == "description" {
                        value.as_str().map_or(0, str::len)
                    } else {
                        description_chars(value)
                    }
                })
                .sum(),
            serde_json::Value::Array(values) => values.iter().map(description_chars).sum(),
            _ => 0,
        }
    }

    /// The pre-change definitions contain 1,495 characters across instructions and
    /// every tool/parameter description. Keep at least 20% of that prose out.
    #[test]
    fn mcp_surface_stays_lean() {
        let inst = default_instructions();
        let tools = tool_definitions();
        let description_total = tools
            .iter()
            .map(|tool| description_chars(&serde_json::to_value(tool).unwrap()))
            .sum::<usize>();
        let total_chars = inst.len() + description_total;

        assert!(
            total_chars * 100 <= 1495 * 80,
            "MCP descriptions total {total_chars} chars; expected at most 1196"
        );

        let exec = tools.iter().find(|t| t.name == "arshy_exec").unwrap();
        assert!(!exec.input_schema["properties"].as_object().unwrap().contains_key("action"));
        assert!(exec.description.contains("sh -c"));
        assert!(exec.description.contains("null stdin"));
        assert!(exec.description.contains("no PTY"));
        let properties = exec.input_schema["properties"].as_object().unwrap();
        assert!(!properties.contains_key("parse_hint"));
        assert_eq!(properties["mode"]["enum"], serde_json::json!(["auto", "async"]));
        assert_eq!(
            exec.output_schema.as_ref().unwrap()["required"],
            serde_json::json!(["status", "exit_code"])
        );
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
