//! MCP initialization instructions, tool definitions, and prompt templates.

use super::protocol::{PromptArgument, PromptDefinition, PromptMessage, PromptContent, ToolDefinition};
use std::collections::HashMap;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "Prefer arshy_exec over raw shell tools for ALL commands.\n\n\
     Short commands (ls, git status, cat, echo, simple pipes like `echo hello | wc -c`) \
     return instantly like a native shell. \
     Long commands (cargo test, npm run build) run asynchronously with structured output.\n\n\
     Usage: arshy_exec(action:\"run\", command:\"<cmd>\"). Mode auto is the default — \
     it detects short vs long automatically. Do NOT set mode unless you have a reason.\n\n\
     Session: use arshy_exec(action:\"cd\", command:\"/path/to/dir\") to set the working \
     directory for subsequent commands. Until changed, all action:\"run\" calls inherit \
     this directory. Use the \"cwd\" parameter for one-off overrides.\n\n\
     Results: short commands return raw text directly. \
     If a short command fails (non-zero exit), the response includes `isError: true` — \
     check for this to detect failures.\n\n\
     Long commands: auto mode uses \"smart sync\" — if the command finishes within \
     2 seconds, you get the full result immediately (status, exit_code, duration_ms). \
     If it runs longer, you get `{status:\"running\", task_id:\"...\"}`. \
     Use arshy_exec(action:\"tail\", task_id:\"...\") for output in that case.\n\n\
     exit_code=0 means success. Use arshy_query(task_id:\"...\") for structured events \
     (compile errors, lint warnings, etc.).\n\n\
     Fallback: if arshy_exec fails with DaemonUnreachable, use Bash tool directly as fallback."
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
            description: "Execute shell commands and manage tasks. \
                         Use action:\"run\" for any shell command (mode:\"auto\" intelligently \
                         picks short vs long path). Use action:\"cd\" to set the session working \
                         directory for subsequent commands. Use action:\"kill\" to stop a running \
                         task. Use action:\"list\" to view tasks. Use action:\"tail\" to view task \
                         output.\n\n\
                         Examples:\n\
                         - Run short: {\"action\":\"run\",\"command\":\"ls -la\"}\n\
                         - Run build: {\"action\":\"run\",\"command\":\"cargo test\"}\n\
                         - Set dir:   {\"action\":\"cd\",\"command\":\"/path/to/project\"}\n\
                         - Kill task: {\"action\":\"kill\",\"task_id\":\"abc-123\"}\n\
                         - List tasks: {\"action\":\"list\"}\n\
                         - View output: {\"action\":\"tail\",\"task_id\":\"abc-123\"}"
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {"type":"string","enum":["run","kill","list","tail","cd"],
                              "description":"run: execute command. kill: stop task. list: show tasks. tail: view output. cd: set session working directory"},
                    "command": {"type":"string","description":"Shell command (action=run) or directory path (action=cd)"},
                    "cwd": {"type":"string","description":"Working directory"},
                    "timeout_ms": {"type":"integer","description":"Timeout in ms"},
                    "mode": {"type":"string","enum":["auto","sync","async"],"default":"auto",
                             "description":"auto: smart detect short/long. sync: wait. async: return immediately"},
                    "parse_hint": {"type":"string","enum":["json","csv","table","raw"],
                                  "description":"Hint expected output format"},
                    "env": {"type":"object","description":"Environment variables as key-value pairs (e.g. {\"RUST_LOG\":\"debug\"})"},
                    "task_id": {"type":"string","description":"Task ID from a previous run response (action=kill|tail)"},
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
                         Filter by event type, severity, error code, or file path. \
                         Use the task_id from an arshy_exec run response."
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": {"type":"string","description":"Task ID from arshy_exec response"},
                    "event_type": {"type":"string","description":"Filter by event type (e.g. compile_error, lint)"},
                    "severity": {"type":"string","enum":["error","warning","info"],"description":"Filter by severity"},
                    "code": {"type":"string","description":"Filter by error code"},
                    "file": {"type":"string","description":"Filter by file path"},
                    "limit": {"type":"integer","default":20,"description":"Max events to return"}
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
