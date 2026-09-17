//! CLI command dispatch — connects to daemon via UDS and executes user commands.

mod daemon;
pub mod render;
mod tasks;
mod update;

pub(crate) use daemon::{daemon_action, daemon_stats, daemon_status, doctor};
pub(crate) use tasks::{
    connect, kill_task, list_tasks, prune, query_events, run_command, tail_task,
};
pub(crate) use update::self_update;

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, Request, METHOD_ANALYZE};
use arshy_lib::Result;
use std::path::PathBuf;

use crate::{Cli, CliCommand, ConfigAction, McpAction, ParserAction};

const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";

/// Route a parsed CLI to the appropriate handler.
pub async fn dispatch(cli: Cli) -> Result<()> {
    let config_path = cli.config;
    let log_level = cli.log_level;

    match cli.command {
        Some(CliCommand::Run { command, cwd, timeout_ms, mode, format, errors_only, purpose }) => {
            run_command(
                config_path,
                log_level,
                &command,
                cwd,
                timeout_ms,
                mode,
                &format,
                errors_only,
                purpose,
            )
            .await
        }
        Some(CliCommand::List { status, limit }) => {
            list_tasks(config_path, log_level, status, limit).await
        }
        Some(CliCommand::Query { task_id, event_type, severity, code, file, limit }) => {
            query_events(config_path, log_level, task_id, event_type, severity, code, file, limit)
                .await
        }
        Some(CliCommand::Kill { task_id }) => kill_task(config_path, log_level, &task_id).await,
        Some(CliCommand::Tail { task_id, lines, format }) => {
            tail_task(config_path, log_level, &task_id, lines, &format).await
        }
        Some(CliCommand::Prune { keep, older_than }) => {
            prune(config_path, log_level, keep, older_than).await
        }
        Some(CliCommand::Config { action }) => config(action, config_path),
        Some(CliCommand::Status) => daemon_status(config_path, log_level).await,
        Some(CliCommand::Daemon { action }) => daemon_action(action, config_path, log_level).await,
        Some(CliCommand::Stats { format }) => daemon_stats(config_path, log_level, &format).await,
        Some(CliCommand::Doctor { format }) => doctor(config_path, log_level, None, &format).await,
        Some(CliCommand::Analyze { format }) => analyze(config_path, log_level, &format).await,
        Some(CliCommand::Parser { action }) => parser_action(action, config_path, log_level).await,
        Some(CliCommand::Mcp { action }) => mcp_action(action, config_path).await,
        Some(CliCommand::SelfUpdate { dest }) => self_update(dest),
        None => {
            println!("Arshy — AI Agent native shell execution layer");
            println!("Usage: arshy [OPTIONS] <COMMAND>");
            Ok(())
        }
    }
}

async fn mcp_action(action: McpAction, config_path: Option<PathBuf>) -> Result<()> {
    match action {
        McpAction::Serve => crate::proxy::run_async(config_path).await,
        McpAction::Config { format } => {
            let command = current_arshy_command()?;
            match format.as_str() {
                "json" => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "command": command,
                        "args": ["mcp", "serve"]
                    }))?
                ),
                "command" => println!("{} mcp serve", command),
                "prompt" => print!("{}", setup_prompt(&command)),
                other => {
                    return Err(arshy_lib::ArshyError::Other(format!(
                        "unsupported MCP config format `{other}`; use json, command, or prompt"
                    )))
                }
            }
            Ok(())
        }
    }
}

fn current_arshy_command() -> Result<String> {
    let path = std::env::current_exe().map_err(|error| {
        arshy_lib::ArshyError::Other(format!("cannot locate arshy executable: {error}"))
    })?;
    let path = path.canonicalize().map_err(|error| {
        arshy_lib::ArshyError::Other(format!("cannot resolve arshy executable path: {error}"))
    })?;
    if path.file_name().and_then(|name| name.to_str()) != Some("arshy") {
        return Err(arshy_lib::ArshyError::Other(format!(
            "mcp config must run from the arshy executable, found {}",
            path.display()
        )));
    }
    Ok(path.to_string_lossy().into_owned())
}

const SETUP_PROMPT_VERSION: &str = "Arshy setup prompt v1";
const SETUP_PROMPT_TEMPLATE: &str = r#"<ARSHY_SETUP_PROMPT_VERSION>

Configure Arshy for the current local coding-agent client.

This prompt was generated on the machine where the executable below exists.
Use it only on that same machine and in the project where configuration is intended.

Authoritative server entry:
{"command": <ARSHY_ABSOLUTE_PATH>, "args": ["mcp", "serve"]}

Use the current client's native MCP management command or API. Do not guess
undocumented configuration formats. Register one stdio MCP server named `arshy`,
preferably at user/global scope; if this client only supports project scope, use
that scope and report it.

Before changing anything:
1. Confirm the authoritative executable exists and invoke it with the single
   argument `doctor` through the available shell path.
2. Inspect the existing MCP entry named `arshy`.
3. Check the current project's instruction hierarchy and follow its existing
   conventions. Do not create or rewrite a project rule file for Arshy unless
   the current project policy and user authorization call for it.

Apply only the minimum change needed:
- If an existing `arshy` entry has the exact command and args above, succeed
  without changing it.
- If an existing `arshy` entry differs, stop and show its current value; never overwrite it.
- Preserve all user content in any project file you are authorized to edit and
  report the file and change.

Safety and recovery:
- Do not modify PATH, shell profiles, hooks, unrelated MCP entries, unrelated
  project rules, or installed software. Do not elevate privileges.
- If a GUI, enterprise policy, missing permission, or user approval is required,
  stop and give the exact manual action.
- If Arshy is unavailable, use the native shell only for `arshy doctor` or
  `arshy daemon restart`, then retry Arshy. Any other fallback must be explicit.

Verification and rollback:
- Verify the registration with the client's native list/get mechanism and run
  `arshy doctor`.
- Report the actual scope, files changed, and whether a restart or new task is
  required. Do not claim the current session has loaded the tool before reload.
- If a later step fails, revert only the MCP entry or project-file change made
  by this run; leave pre-existing values untouched and report rollback failures.
"#;

fn setup_prompt(command: &str) -> String {
    let command = serde_json::to_string(command).expect("a string is always JSON serializable");
    SETUP_PROMPT_TEMPLATE
        .replace("<ARSHY_SETUP_PROMPT_VERSION>", SETUP_PROMPT_VERSION)
        .replace("<ARSHY_ABSOLUTE_PATH>", &command)
}

fn config(action: ConfigAction, config_path: Option<PathBuf>) -> Result<()> {
    let overrides =
        arshy_lib::config::CliOverrides { config_path: config_path.clone(), ..Default::default() };
    match action {
        ConfigAction::Get { key } => config_get(&key, &overrides),
        ConfigAction::List => {
            let cfg = Config::load(overrides)?;
            println!("{}", toml::to_string_pretty(&cfg)?);
            Ok(())
        }
        ConfigAction::Path => {
            match config_path {
                Some(p) => println!("{}", p.display()),
                None => match arshy_lib::config::default_config_path() {
                    Some(p) => println!("{}", p.display()),
                    None => println!("(no default config path)"),
                },
            }
            Ok(())
        }
    }
}

/// Get a config value by dot-separated key path.
fn config_get(key: &str, overrides: &arshy_lib::config::CliOverrides) -> Result<()> {
    let cfg = Config::load(overrides.clone())?;
    let value = serde_json::to_value(&cfg)?;
    match navigate_json(&value, key) {
        Some(v) => {
            if let Some(s) = v.as_str() {
                println!("{}", s);
            } else {
                println!("{}", serde_json::to_string_pretty(v)?);
            }
        }
        None => {
            eprintln!("unknown key: {}", key);
            eprintln!("valid keys: {}", VALID_KEYS.join(", "));
            std::process::exit(1);
        }
    }
    Ok(())
}

/// Navigate a JSON value by dot-separated path.
fn navigate_json<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

const VALID_KEYS: &[&str] = &[
    "daemon.socket_path",
    "daemon.log_level",
    "daemon.log_format",
    "daemon.auto_start",
    "daemon.max_task_duration_ms",
    "daemon.max_output_bytes",
    "daemon.kill_graceful_ms",
    "daemon.kill_force_ms",
    "daemon.max_concurrent_tasks",
    "daemon.idle_timeout_secs",
    "store.store_dir",
    "store.integrity_check",
    "store.auto_prune",
    "store.prune_keep",
    "store.prune_older_than_days",
    "parser.dirs",
    "parser.hot_reload",
    "notifications.batch_interval_ms",
    "notifications.max_batch_events",
];

async fn parser_action(
    action: ParserAction,
    config_path: Option<PathBuf>,
    log_level: Option<String>,
) -> Result<()> {
    match action {
        ParserAction::Reload => parser_reload(config_path, log_level).await,
        ParserAction::List => parser_list(config_path, log_level).await,
        ParserAction::Benchmark => run_benchmark(),
    }
}

async fn parser_reload(config_path: Option<PathBuf>, log_level: Option<String>) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: ipc::METHOD_PARSER_RELOAD.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;

    let diff = response.result.get("diff").and_then(|v| v.as_str()).unwrap_or("");

    if diff.is_empty() {
        eprintln!("  {}✓ Parsers reloaded — no changes{}", GREEN, RESET);
    } else {
        eprintln!("  {}✓ Parsers reloaded — changes detected:{}", GREEN, RESET);
        eprintln!();
        for line in diff.lines() {
            if line.starts_with('+') {
                eprintln!("    {}{}{}", GREEN, line, RESET);
            } else if line.starts_with('-') {
                eprintln!("    {}{}{}", RED, line, RESET);
            } else {
                eprintln!("    {}{}{}", YELLOW, line, RESET);
            }
        }
    }
    eprintln!();
    Ok(())
}

async fn parser_list(config_path: Option<PathBuf>, log_level: Option<String>) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: ipc::METHOD_STATUS.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;

    let parser_count = response.result.get("parser_count").and_then(|v| v.as_u64()).unwrap_or(0);

    eprintln!("  Loaded parsers: {}", parser_count);
    eprintln!();
    Ok(())
}

// ── Benchmark ──────────────────────────────────────────────────────────────

/// Run parser benchmark by executing the daemon's benchmark test.
/// The test outputs structured JSON to stderr between marker lines.
fn run_benchmark() -> Result<()> {
    eprintln!("Running benchmark across all builtin parsers...\n");

    let output = std::process::Command::new("cargo")
        .args(["test", "--lib", "daemon::parser::benchmark::run_benchmark", "--", "--nocapture"])
        .output()
        .map_err(|e| arshy_lib::ArshyError::Other(format!("failed to run cargo test: {}", e)))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}\n{}", stdout, stderr);

    // Extract JSON between markers
    let json_str = combined
        .lines()
        .skip_while(|l| !l.contains("=== ARSHY BENCHMARK ==="))
        .skip(1) // skip the marker itself
        .take_while(|l| !l.contains("=== END BENCHMARK ==="))
        .collect::<Vec<_>>()
        .join("\n");

    if json_str.trim().is_empty() {
        eprintln!("ERROR: Benchmark produced no output.");
        if !output.status.success() {
            eprintln!("cargo test failed. Output:\n{}", combined);
        }
        return Err(arshy_lib::ArshyError::Other("benchmark produced no output".into()));
    }

    // Parse and display
    let mut result: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| arshy_lib::ArshyError::Other(format!("invalid benchmark JSON: {}", e)))?;

    let report_path = if std::path::Path::new("docs").is_dir() {
        std::path::PathBuf::from("docs/benchmark_report.json")
    } else {
        std::path::PathBuf::from(".arshy-benchmark.json")
    };

    if let Ok(baseline_str) = std::fs::read_to_string(&report_path) {
        if let Ok(baseline) = serde_json::from_str::<serde_json::Value>(&baseline_str) {
            let mut diff = serde_json::json!({});

            if let (Some(new_val), Some(old_val)) = (
                result.get("avg_accuracy").and_then(|v| v.as_f64()),
                baseline.get("avg_accuracy").and_then(|v| v.as_f64()),
            ) {
                diff["avg_accuracy"] = serde_json::json!(new_val - old_val);
            }
            if let (Some(new_val), Some(old_val)) = (
                result.get("error_speed_advantage_pct").and_then(|v| v.as_f64()),
                baseline.get("error_speed_advantage_pct").and_then(|v| v.as_f64()),
            ) {
                diff["error_speed_advantage_pct"] = serde_json::json!(new_val - old_val);
            }
            if let (Some(new_val), Some(old_val)) = (
                result.get("total_unparsed_error_lines").and_then(|v| v.as_i64()),
                baseline.get("total_unparsed_error_lines").and_then(|v| v.as_i64()),
            ) {
                diff["total_unparsed_error_lines"] = serde_json::json!(new_val - old_val);
            }

            if let (Some(new_details), Some(old_details)) = (
                result.get_mut("details").and_then(|v| v.as_array_mut()),
                baseline.get("details").and_then(|v| v.as_array()),
            ) {
                for detail in new_details.iter_mut() {
                    let parser = detail.get("parser").and_then(|v| v.as_str()).unwrap_or("");
                    let fixture = detail.get("fixture").and_then(|v| v.as_str()).unwrap_or("");

                    if let Some(old_detail) = old_details.iter().find(|d| {
                        d.get("parser").and_then(|v| v.as_str()) == Some(parser)
                            && d.get("fixture").and_then(|v| v.as_str()) == Some(fixture)
                    }) {
                        if let (Some(new_acc), Some(old_acc)) = (
                            detail.get("accuracy").and_then(|v| v.as_f64()),
                            old_detail.get("accuracy").and_then(|v| v.as_f64()),
                        ) {
                            detail["accuracy_diff"] = serde_json::json!(new_acc - old_acc);
                        }
                        if let (Some(new_unparsed), Some(old_unparsed)) = (
                            detail.get("unparsed_error_lines").and_then(|v| v.as_i64()),
                            old_detail.get("unparsed_error_lines").and_then(|v| v.as_i64()),
                        ) {
                            detail["unparsed_diff"] =
                                serde_json::json!(new_unparsed - old_unparsed);
                        }
                    }
                }
            }

            result["comparison"] = diff;
        }
    }

    let _ = std::fs::write(&report_path, &json_str);

    eprint!("{}", render::render_benchmark(&result));

    Ok(())
}

async fn analyze(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    format: &str,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_ANALYZE.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;

    match format {
        "pretty" => {
            eprint!("{}", render::render_analyze(&response.result));
        }
        _ => {
            // Default: JSON for agent consumption
            println!("{}", serde_json::to_string_pretty(&response.result)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{setup_prompt, update::default_install_dir, SETUP_PROMPT_VERSION};

    #[test]
    fn default_install_dir_dev_build_uses_local_bin() {
        let exe = std::path::Path::new("/Users/x/repo/target/release/arshy");
        let dir = default_install_dir(exe);
        assert!(dir.ends_with(".local/bin"), "dev build should target ~/.local/bin, got {dir:?}");
    }

    #[test]
    fn default_install_dir_installed_uses_own_dir() {
        let exe = std::path::Path::new("/Users/x/.local/bin/arshy");
        let dir = default_install_dir(exe);
        assert_eq!(dir, std::path::Path::new("/Users/x/.local/bin"));
    }

    #[test]
    fn default_install_dir_tmp_build_uses_own_dir() {
        let exe = std::path::Path::new("/opt/arshy/builds/v0.1.0/arshy");
        let dir = default_install_dir(exe);
        assert_eq!(dir, std::path::Path::new("/opt/arshy/builds/v0.1.0"));
    }

    #[test]
    fn setup_prompt_is_client_neutral_and_contains_safety_contract() {
        let prompt = setup_prompt("/tmp/project with spaces/arshy");
        assert!(prompt.starts_with(SETUP_PROMPT_VERSION));
        assert!(prompt.contains(r#""command": "/tmp/project with spaces/arshy"#));
        assert!(prompt.contains(r#""args": ["mcp", "serve"]"#));
        assert!(!prompt.contains("<ARSHY_ABSOLUTE_PATH>"));
        assert!(!prompt.contains("<ARSHY_SETUP_PROMPT_VERSION>"));
        assert!(prompt.contains("same machine"));
        assert!(prompt.contains("project policy"));
        assert!(prompt.contains("never overwrite it"));
        assert!(prompt.contains("Do not elevate privileges"));
        assert!(prompt.contains("Verification and rollback"));
        assert!(prompt.contains("new task"));
        assert!(!prompt.contains("codex mcp add"));
        assert!(!prompt.contains("claude mcp add"));
        assert!(!prompt.contains("gemini mcp add"));
        assert!(!prompt.contains(".cursor/mcp.json"));
        assert!(!prompt.contains("arshy:managed:start"));
        assert!(!prompt.contains("arshy:managed:end"));
        assert!(!prompt.contains("```"));
    }

    #[test]
    fn setup_prompt_json_escapes_special_executable_paths() {
        let prompt = setup_prompt("/tmp/quoted\\path/\"arshy");
        assert!(prompt.contains(r#"/tmp/quoted\\path/\"arshy"#));
        assert!(prompt.contains("{\"command\": "));
    }
}
