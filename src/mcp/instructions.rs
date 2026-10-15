//! MCP initialization instructions and tool definitions.

use super::protocol::ToolDefinition;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "Use arshy_exec for commands (sh -c, null stdin, no PTY). Default replies are compact \
     diagnostics; fetch full captured output with arshy_task raw. Use arshy_query to page \
     saved events."
        .into()
}

/// Return the complete list of MCP tool definitions.
///
/// Three single-purpose tools: execute, query diagnostics, and low-frequency
/// task lifecycle operations. Keep schemas lean — host agents inject these on
/// every turn.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "arshy_exec".into(),
            description: "Run one sh -c command (null stdin, no PTY). Returns compact diagnostics \
                         on failure; use arshy_task raw for full output. mode=auto|async."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type":"string"},
                    "cwd": {"type":"string"},
                    "timeout_ms": {"type":"integer"},
                    "mode": {"type":"string","enum":["auto","async"],"default":"auto"},
                    "env": {"type":"object"}
                },
                "required": ["command"]
            }),
            // Omit outputSchema: hosts already receive structuredContent, and schemas
            // are re-injected into every model turn.
            output_schema: None,
        },
        ToolDefinition {
            name: "arshy_query".into(),
            description: "Search saved diagnostics. Prefer over dumping raw logs.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string"},
                    "event_type": {"type":"string"},
                    "severity": {"type":"string","enum":["error","warning","info"]},
                    "code": {"type":"string"},
                    "file": {"type":"string"},
                    "limit": {"type":"integer","default":20},
                    "offset": {"type":"integer","default":0}
                },
                "required": []
            }),
            output_schema: None,
        },
        ToolDefinition {
            name: "arshy_task".into(),
            description: "cancel | list | raw (captured output).".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {"type":"string","enum":["cancel","list","raw"]},
                    "task_id": {"type":"string"},
                    "lines": {"type":"integer","default":200},
                    "status": {"type":"string","enum":["running","completed","failed","timeout","killed"]},
                    "limit": {"type":"integer","default":10}
                },
                "required": ["action"]
            }),
            output_schema: None,
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
        assert!(inst.contains("arshy_task"));
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

    /// Pre-slim surface was ~1,495 description chars. Keep well under half.
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
            total_chars <= 600,
            "MCP descriptions total {total_chars} chars; expected at most 600"
        );

        let exec = tools.iter().find(|t| t.name == "arshy_exec").unwrap();
        assert!(!exec.input_schema["properties"].as_object().unwrap().contains_key("action"));
        assert!(exec.description.contains("sh -c") || exec.description.contains("compact"));
        assert!(exec.output_schema.is_none());
        let properties = exec.input_schema["properties"].as_object().unwrap();
        assert!(!properties.contains_key("parse_hint"));
        assert_eq!(properties["mode"]["enum"], serde_json::json!(["auto", "async"]));
        // Property descriptions stripped to save host context tokens.
        assert!(properties["command"].get("description").is_none());
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
            assert!(t.output_schema.is_none(), "tool {} should omit outputSchema", t.name);
            assert_eq!(
                t.input_schema.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "tool {} input_schema must be an object",
                t.name
            );
        }
    }
}
