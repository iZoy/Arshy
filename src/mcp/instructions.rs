//! MCP initialization instructions, tool definitions, and prompt templates.

use super::protocol::{
    PromptArgument, PromptContent, PromptDefinition, PromptMessage, ToolDefinition,
};
use std::collections::HashMap;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "arshy_exec is your shell for ALL command execution. Route every shell command through \
     arshy_exec(action:\"run\", command:\"<cmd>\") — do not use raw Bash unless arshy is unreachable.\n\n\
     mode:\"auto\" (the default) handles everything: short commands (ls, grep, git status) return \
     text instantly. Long commands (cargo test, npm run build) wait for completion and return \
     structured results with diagnostics (file/line/code fields). Run command, get result, make decision.\n\n\
     Session directory: use arshy_exec(action:\"cd\", command:\"/absolute/path\") once; all \
     subsequent run calls inherit that directory. Use the \"cwd\" parameter for one-off overrides.\n\n\
     The result always contains: status, exit_code, duration_ms, events[] (structured diagnostics). \
     You never need to manage task IDs, subscribe, or poll.\n\n\
     Fallback: if arshy_exec returns DaemonUnreachable, use Bash directly as a one-off fallback."
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
            description: "Your primary shell for ALL command execution. \
                         mode:\"auto\" (the default) runs commands and returns complete results: \
                         short commands (ls, grep) return text instantly, \
                         long commands (cargo test, npm run build) wait for completion and return \
                         structured output with diagnostics (file/line/code fields). \
                         You don't manage task IDs or subscribe — just run and get results.\n\n\
                         Other actions: \"cd\" sets session working directory, \"kill\" stops \
                         a running task, \"list\" shows recent tasks, \"tail\" views task output.\n\n\
                         Examples:\n\
                         - arshy_exec(action:\"run\", command:\"cargo test\")\n\
                         - arshy_exec(action:\"run\", command:\"grep -rn foo src/\")\n\
                         - arshy_exec(action:\"cd\", command:\"/path/to/project\")"
                .into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {"type":"string","enum":["run","kill","list","tail","cd","subscribe"],
                              "description":"run: execute command. kill: stop task. list: show tasks. tail: view output. cd: set session working directory. subscribe: wait for task completion"},
                    "command": {"type":"string","description":"Shell command (action=run) or directory path (action=cd)"},
                    "cwd": {"type":"string","description":"Absolute path for working directory (one-off override; use action:cd for session-wide)"},
                    "timeout_ms": {"type":"integer","description":"Timeout in ms"},
                    "mode": {"type":"string","enum":["auto","sync","async"],"default":"auto",
                             "description":"auto: smart detect short/long. sync: wait. async: return immediately"},
                    "parse_hint": {"type":"string",
                                  "description":"Expected output format (json|csv|table|raw) or parser name to force (e.g. \"python\", \"cargo\")"},
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
                         Returns typed events (diagnostic, location, test_result, crash, summary) \
                         with severity, code, file path, and line number — no regex parsing needed. \
                         Filter by event_type, severity, error code, or file path. \
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
                    description:
                        "The build output or error log (optional, can also query by task_id)".into(),
                    required: false,
                },
            ],
        },
        PromptDefinition {
            name: "diagnose_test_failure".into(),
            description: "Help me understand why these tests failed and suggest next steps.".into(),
            arguments: vec![PromptArgument {
                name: "task_id".into(),
                description: "The task ID of the failed test run".into(),
                required: true,
            }],
        },
        PromptDefinition {
            name: "review_task_output".into(),
            description: "Review the structured output of a command and summarize findings.".into(),
            arguments: vec![PromptArgument {
                name: "task_id".into(),
                description: "The task ID to review".into(),
                required: true,
            }],
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
                 3. Suggest concrete fixes for each error",
            );
            Some(vec![PromptMessage { role: "user".into(), content: PromptContent::Text { text } }])
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
