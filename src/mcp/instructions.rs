//! MCP initialization instructions, tool definitions, and prompt templates.

use super::protocol::{
    PromptArgument, PromptContent, PromptDefinition, PromptMessage, ToolDefinition,
};
use std::collections::HashMap;

/// Return the default MCP instructions (plain string per MCP spec).
pub fn default_instructions() -> String {
    "arshy_exec is your shell for ALL command execution. Route every shell command through \
     arshy_exec(action:\"run\", command:\"<cmd>\") — do not use raw Bash unless arshy is unreachable.\n\n\
     Why: arshy automatically detects short vs long commands (mode:\"auto\" is the default). \
     Short commands (ls, git status, grep, cat, echo) return text instantly like a native shell. \
     Long commands (cargo test, npm run build, pytest) run asynchronously with structured output — \
     compiler errors become typed diagnostic events with file/line/code fields, not raw text you \
     must regex-parse yourself.\n\n\
     You never need to think about \"should this be sync or async?\" — mode auto handles it.\n\n\
     Session directory: use arshy_exec(action:\"cd\", command:\"/absolute/path\") once; all \
     subsequent run calls inherit that directory. Use the \"cwd\" parameter for one-off overrides.\n\n\
     Results: short commands return raw text. If they fail, isError is true. Long commands \
     return structured JSON: {status, exit_code, duration_ms, task_id, event_count}. \
     If a long command runs >2s, you get {status:\"running\", task_id:\"...\"} — then call \
     arshy_exec(action:\"subscribe\", task_id:\"...\") to block until it finishes. \
     No polling needed.\n\n\
     Query structured events: arshy_query(task_id:\"...\") returns typed events \
     (diagnostic, location, test_result, crash) with severity, code, file, line. \
     Filter by event_type, severity, or file path.\n\n\
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
                         Use action:\"run\" with mode:\"auto\" (the default) — it intelligently \
                         picks the right execution path: short commands return instantly, \
                         long commands (builds, tests) run async with structured output \
                         (diagnostics with file/line/code fields, not raw text). \
                         Other actions: \"cd\" sets session working directory, \"kill\" stops \
                         a running task, \"list\" shows recent tasks, \"tail\" views task output, \
                         \"subscribe\" blocks until a task completes.\n\n\
                         Examples:\n\
                         - arshy_exec(action:\"run\", command:\"cargo test\")\n\
                         - arshy_exec(action:\"run\", command:\"grep -rn foo src/\")\n\
                         - arshy_exec(action:\"cd\", command:\"/path/to/project\")\n\
                         - arshy_exec(action:\"subscribe\", task_id:\"abc-123\")"
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
