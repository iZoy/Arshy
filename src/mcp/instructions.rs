//! MCP initialization instructions and tool definitions.

use super::protocol::ToolDefinition;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "Route every shell command through arshy_exec(action:\"run\", command:\"<cmd>\") instead of \
     raw Bash — fall back to Bash only when arshy is unreachable.\n\n\
     mode:\"auto\" (default) returns complete results in one call: short commands come back as \
     instant text, long commands as structured diagnostics (file/line/code). Don't manage task IDs \
     or poll.\n\n\
     Session directory: arshy_exec(action:\"cd\", command:\"/absolute/path\") once; later runs \
     inherit it (use \"cwd\" for one-off overrides)."
        .into()
}

/// Return the complete list of MCP tool definitions.
///
/// 2-tool model: `arshy_exec` (unified run/cd/kill/list/tail) + `arshy_query` (events).
/// Reduces ~60% tool definition tokens and improves agent selection accuracy.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "arshy_exec".into(),
            description: "Run shell commands and get complete results in one call. \
                         mode:\"auto\" (default): short commands (ls, grep) return text instantly; \
                         long commands (cargo test, npm run build) return structured diagnostics \
                         (file/line/code).\n\n\
                         Actions: run (default) | cd (session dir) | kill | list | tail (event view) | \
                         raw (original output, lines=0 for all) | subscribe.\n\n\
                         Examples:\n\
                         - arshy_exec(action:\"run\", command:\"cargo test\")\n\
                         - arshy_exec(action:\"raw\", task_id:\"<id>\")"
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {"type":"string","enum":["run","kill","list","tail","raw","cd","subscribe"],
                              "description":"run: execute. cd: set session directory. kill: stop a task. list: show tasks. tail: event output. raw: original output (lines=0 all). subscribe: wait for completion"},
                    "command": {"type":"string","description":"Shell command (action=run) or directory path (action=cd)"},
                    "cwd": {"type":"string","description":"Absolute path for working directory (one-off override; use action:cd for session-wide)"},
                    "timeout_ms": {"type":"integer","description":"Timeout in ms"},
                    "mode": {"type":"string","enum":["auto","sync","async"],"default":"auto",
                             "description":"auto: smart detect short/long. sync: wait. async: return immediately"},
                    "parse_hint": {"type":"string",
                                  "description":"Force a parser by name (e.g. \"python\", \"cargo\", \"raw\"). JSON output is auto-detected by the pipeline; no csv/table hints exist"},
                    "env": {"type":"object","description":"Environment variables as key-value pairs (e.g. {\"RUST_LOG\":\"debug\"})"},
                    "task_id": {"type":"string","description":"Task ID from a previous run response (action=kill|tail|raw)"},
                    "lines": {"type":"integer","default":50,"description":"Lines (action=tail; action=raw defaults to 200, 0 = all)"},
                    "format": {"type":"string","enum":["event","raw"],"default":"event",
                              "description":"Output format (action=tail)"},
                    "status": {"type":"string","enum":["running","completed","failed","killed"],
                              "description":"Filter by status (action=list)"},
                    "limit": {"type":"integer","default":10,"description":"Max results (action=list)"}
                },
                "required": ["action"]
            }),
        },
        ToolDefinition {
            name: "arshy_query".into(),
            description: "Query structured events from one task, or search across ALL tasks. \
                         Pass task_id to inspect one task; omit it to search history (results \
                         carry task_id). Filter by event_type, severity, code, or file."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string","description":"Task ID from arshy_exec response. Omit to search across all tasks."},
                    "event_type": {"type":"string","description":"Filter by event type (e.g. compile_error, lint)"},
                    "severity": {"type":"string","enum":["error","warning","info"],"description":"Filter by severity"},
                    "code": {"type":"string","description":"Filter by error code"},
                    "file": {"type":"string","description":"Filter by file path"},
                    "limit": {"type":"integer","default":20,"description":"Max events to return"}
                },
                "required": []
            }),
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
        let action_desc = exec
            .input_schema
            .get("properties")
            .and_then(|p| p.get("action"))
            .and_then(|a| a.get("description"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let total_chars =
            inst.len() + exec.description.len() + query.description.len() + action_desc.len();

        assert!(
            total_chars <= 1600,
            "MCP surface grew to {total_chars} chars (~{} tokens) — trim before merging",
            total_chars / 4
        );

        // No duplicated action enumeration between instructions and the tool
        // description: the action list belongs in exactly one place.
        assert!(!inst.contains("subscribe"), "action list belongs in the tool description");
        assert!(!inst.contains("kill"), "action list belongs in the tool description");
    }

    #[test]
    fn tool_definitions_have_expected_tools() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 2);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"arshy_exec"));
        assert!(names.contains(&"arshy_query"));
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
