//! MCP initialization instructions, tool definitions, and prompt templates.

use super::protocol::{PromptArgument, PromptDefinition, PromptMessage, PromptContent, ToolDefinition};
use std::collections::HashMap;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "NEVER use raw shell tools. ALL commands go through arshy_exec action:\"run\". \
     Short commands (ls, git status, echo) return instantly like a native shell. \
     Long commands (builds, tests, installs) stream structured output.\n\n\
     Use mode:\"auto\" (the default). Arshy automatically detects short vs long commands: \
     short commands return raw text instantly with zero overhead; \
     long commands run asynchronously with structured events and real-time notifications. \
     Do NOT manually set mode to \"sync\" or \"async\" unless you have a specific reason."
        .into()
}

/// Return the complete list of MCP tool definitions.
///
/// 2-tool model: `arshy_exec` (unified run/kill/list/tail) + `arshy_query` (events).
/// Reduces ~60% tool definition tokens and improves agent selection accuracy.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "arshy_exec".into(),
            description: "Execute shell commands and manage tasks. \
                         Use action:\"run\" for any shell command (mode:\"auto\" intelligently \
                         picks short vs long path). Use action:\"kill\" to stop a running task. \
                         Use action:\"list\" to view tasks. Use action:\"tail\" to view task output. \
                         NEVER use raw shell tools — always use arshy_exec."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {"type":"string","enum":["run","kill","list","tail"],
                              "description":"run: execute command. kill: stop task. list: show tasks. tail: view output"},
                    "command": {"type":"string","description":"Shell command (action=run only)"},
                    "cwd": {"type":"string","description":"Working directory"},
                    "timeout_ms": {"type":"integer","description":"Timeout in ms"},
                    "mode": {"type":"string","enum":["auto","sync","async"],"default":"auto",
                             "description":"auto: smart detect short/long. sync: wait. async: return immediately"},
                    "parse_hint": {"type":"string","enum":["json","csv","table","raw"],
                                  "description":"Hint expected output format"},
                    "task_id": {"type":"string","description":"Task ID (action=kill|tail)"},
                    "lines": {"type":"integer","default":50,"description":"Lines (action=tail)"},
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
            description: "Query structured events from a completed or running task. \
                         Filter by event type, severity, error code, or file path."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string"},
                    "event_type": {"type":"string"},
                    "severity": {"type":"string","enum":["error","warning","info"]},
                    "code": {"type":"string"},
                    "file": {"type":"string"},
                    "limit": {"type":"integer","default":20}
                },
                "required": ["task_id"]
            }),
        },
    ]
}

/// Return all available prompt definitions.
pub fn prompt_definitions() -> Vec<PromptDefinition> {
    vec![
        PromptDefinition {
            name: "analyze_build_failure".into(),
            description: "Help me understand why this build failed and suggest fixes.".into(),
            arguments: vec![
                PromptArgument {
                    name: "command".into(),
                    description: "The build command that failed (e.g. 'cargo build')".into(),
                    required: true,
                },
                PromptArgument {
                    name: "output".into(),
                    description: "The build output or error log (optional, can also query by task_id)".into(),
                    required: false,
                },
            ],
        },
        PromptDefinition {
            name: "diagnose_test_failure".into(),
            description: "Help me understand why these tests failed and suggest next steps.".into(),
            arguments: vec![
                PromptArgument {
                    name: "task_id".into(),
                    description: "The task ID of the failed test run".into(),
                    required: true,
                },
            ],
        },
        PromptDefinition {
            name: "review_task_output".into(),
            description: "Review the structured output of a command and summarize findings.".into(),
            arguments: vec![
                PromptArgument {
                    name: "task_id".into(),
                    description: "The task ID to review".into(),
                    required: true,
                },
            ],
        },
    ]
}

/// Get a prompt's messages by name and arguments.
/// Returns `None` if the prompt name is unknown.
pub fn get_prompt(name: &str, args: &HashMap<String, String>) -> Option<Vec<PromptMessage>> {
    match name {
        "analyze_build_failure" => {
            let cmd = args.get("command").map(|s| s.as_str()).unwrap_or("unknown");
            let output = args.get("output").map(|s| s.as_str()).unwrap_or("");
            let mut text = format!(
                "I need help analyzing a build failure.\n\n\
                 Command: `{}`\n",
                cmd
            );
            if !output.is_empty() {
                text.push_str(&format!("\nBuild output:\n```\n{}\n```\n", output));
            }
            text.push_str(
                "\nPlease:\n\
                 1. Identify the root cause of the failure\n\
                 2. List the specific errors and their locations\n\
                 3. Suggest concrete fixes for each error"
            );
            Some(vec![PromptMessage {
                role: "user".into(),
                content: PromptContent::Text { text },
            }])
        }
        "diagnose_test_failure" => {
            let task_id = args.get("task_id").map(|s| s.as_str()).unwrap_or("unknown");
            Some(vec![PromptMessage {
                role: "user".into(),
                content: PromptContent::Text {
                    text: format!(
                        "I need help diagnosing test failures for task `{}`.\n\n\
                         Please:\n\
                         1. Query the task events (errors and warnings)\n\
                         2. Identify which tests failed and why\n\
                         3. Suggest fixes for each failing test",
                        task_id
                    ),
                },
            }])
        }
        "review_task_output" => {
            let task_id = args.get("task_id").map(|s| s.as_str()).unwrap_or("unknown");
            Some(vec![PromptMessage {
                role: "user".into(),
                content: PromptContent::Text {
                    text: format!(
                        "Review the structured output of task `{}`.\n\n\
                         Please:\n\
                         1. Query the task events\n\
                         2. Summarize the key findings (errors, warnings, diagnostics)\n\
                         3. Highlight anything that needs attention",
                        task_id
                    ),
                },
            }])
        }
        _ => None,
    }
}
