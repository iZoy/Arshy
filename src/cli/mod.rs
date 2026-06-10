//! CLI command dispatch — connects to daemon via UDS and executes user commands.

pub mod render;

use arshy_lib::config::Config;
use arshy_lib::ipc::{
    self, QueryParams, Request, RunTaskParams, METHOD_LIST, METHOD_PRUNE, METHOD_RUN,
    METHOD_SHUTDOWN, METHOD_STATS, METHOD_STATUS,
};
use arshy_lib::Result;
use std::path::PathBuf;

use crate::{Cli, CliCommand, ConfigAction, DaemonAction, ParserAction};

// ANSI color constants for terminal output
const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";

/// Route a parsed CLI to the appropriate handler.
pub async fn dispatch(cli: Cli) -> Result<()> {
    let config_path = cli.config;
    let log_level = cli.log_level;

    match cli.command {
        Some(CliCommand::Run { command, cwd, timeout_ms, mode, format, errors_only }) => {
            run_command(
                config_path,
                log_level,
                &command,
                cwd,
                timeout_ms,
                mode,
                &format,
                errors_only,
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
        Some(CliCommand::Install) => install(),
        Some(CliCommand::Uninstall) => uninstall(),
        Some(CliCommand::Prune { keep, older_than }) => {
            prune(config_path, log_level, keep, older_than).await
        }
        Some(CliCommand::Config { action }) => config(action, config_path),
        Some(CliCommand::Status) => daemon_status(config_path, log_level).await,
        Some(CliCommand::Daemon { action }) => daemon_action(action, config_path, log_level).await,
        Some(CliCommand::Stats) => daemon_stats(config_path, log_level).await,
        Some(CliCommand::InstallLaunchd) => install_launchd(),
        Some(CliCommand::InstallSystemd) => install_systemd(),
        Some(CliCommand::Doctor) => doctor(config_path, log_level),
        Some(CliCommand::Benchmark) => run_benchmark(),
        Some(CliCommand::Parser { action }) => parser_action(action, config_path, log_level).await,
        None => {
            println!("Arshy — AI Agent native shell execution layer");
            println!("Usage: arshy [--from-mcp] [OPTIONS] <COMMAND>");
            Ok(())
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

async fn connect(
    config_path: Option<PathBuf>,
    _log_level: Option<String>,
) -> Result<tokio::net::UnixStream> {
    let cfg = Config::load(arshy_lib::config::CliOverrides { config_path, ..Default::default() })
        .unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();
    ipc::connect(&socket_path).await
}

// ── Subcommand handlers ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_command(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    command: &str,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    mode: Option<String>,
    format: &str,
    errors_only: bool,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let params = RunTaskParams {
        command: command.into(),
        cwd,
        timeout_ms,
        mode: mode.unwrap_or_else(|| "sync".into()),
        parse_hint: None,
        env: None,
        errors_only,
    };
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_RUN.into(),
        params: serde_json::to_value(&params)?,
    };
    let response = ipc::send_request(&mut daemon, &request).await?;

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&response.result)?);
        }
        "pretty" => {
            render::render(&response.result);
        }
        _ => {
            // Default: pretty for interactive, json for non-interactive
            if atty_is_available() {
                render::render(&response.result);
            } else {
                println!("{}", serde_json::to_string_pretty(&response.result)?);
            }
        }
    }
    Ok(())
}

/// Check if stdout is a terminal (for auto-detecting format).
fn atty_is_available() -> bool {
    std::io::IsTerminal::is_terminal(&std::io::stderr())
}

async fn list_tasks(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    status: Option<String>,
    limit: usize,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_LIST.into(),
        params: serde_json::json!({ "status": status, "limit": limit }),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn query_events(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    task_id: String,
    event_type: Option<String>,
    severity: Option<String>,
    code: Option<String>,
    file: Option<String>,
    limit: usize,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let params = QueryParams { task_id, event_type, severity, code, file, limit, offset: 0 };
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: ipc::METHOD_QUERY.into(),
        params: serde_json::to_value(&params)?,
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

async fn kill_task(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    task_id: &str,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: ipc::METHOD_KILL.into(),
        params: serde_json::json!({ "task_id": task_id }),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

async fn tail_task(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    task_id: &str,
    lines: usize,
    format: &str,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: ipc::METHOD_TAIL.into(),
        params: serde_json::json!({ "task_id": task_id, "lines": lines, "format": format }),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

/// Required permission entries for seamless arshy integration.
const ARSHY_PERMISSIONS: &[&str] = &[
    // MCP tools — auto-approve arshy_exec and arshy_query
    "mcp__arshy__arshy_exec",
    "mcp__arshy__arshy_query",
    // Bash tool — auto-approve arshy CLI commands
    "Bash(arshy *)",
    "Bash(arshyd *)",
];

fn install() -> Result<()> {
    println!("Registering arshy as MCP server + permissions...");

    // Write MCP server to ~/.claude.json (Claude Code's actual config)
    let claude_json_path =
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".claude.json");
    install_mcp_in_claude_json(&claude_json_path)?;

    // Write permissions to ~/.claude/settings.json
    let settings_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".claude")
        .join("settings.json");
    install_settings_permissions(&settings_path)?;

    // Auto-start daemon so arshy is immediately usable
    let cfg = Config::load(arshy_lib::config::CliOverrides::default()).unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();
    if socket_path.exists() {
        println!("  ✓ daemon is already running");
    } else {
        match std::process::Command::new("arshyd")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                // Wait up to 3s for the socket to appear
                let mut started = false;
                for _ in 0..15 {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    if socket_path.exists() {
                        started = true;
                        break;
                    }
                }
                if started {
                    println!("  ✓ daemon started");
                } else {
                    println!("  ⚠ daemon spawned but socket not ready — may need a moment");
                }
            }
            Err(e) => {
                println!("  ⚠ could not start daemon: {}", e);
                println!("    Start manually with: arshy daemon start");
            }
        }
    }

    // Hint about launchd/systemd for auto-start on reboot
    if cfg!(target_os = "macos") {
        println!();
        println!("Tip: run `arshy install-launchd` to auto-start daemon on login.");
    }

    println!();
    println!("Done! Restart Claude Code to activate.");
    println!("  MCP server: {}", claude_json_path.display());
    println!("  Permissions: {}", settings_path.display());
    Ok(())
}

/// Write arshy MCP entry to ~/.claude.json under top-level mcpServers (user scope).
fn install_mcp_in_claude_json(path: &std::path::Path) -> Result<()> {
    let mut data: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Find the installed arshy binary (prefer installed over debug/current_exe)
    let arshy_bin = find_installed_binary("arshy")
        .or_else(|| {
            std::env::current_exe().ok().filter(|p| !p.to_string_lossy().contains("target"))
        })
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "arshy".into());

    let arshy_entry = serde_json::json!({
        "type": "stdio",
        "command": arshy_bin,
        "args": ["--from-mcp"],
        "env": {}
    });

    if let Some(obj) = data.as_object_mut() {
        let servers = obj.entry("mcpServers").or_insert_with(|| serde_json::json!({}));
        if let Some(map) = servers.as_object_mut() {
            let changed = map.get("arshy").is_none()
                || map.get("arshy").and_then(|v| v.get("command"))
                    != Some(&serde_json::json!(arshy_bin.clone()));
            map.insert("arshy".into(), arshy_entry);
            if changed {
                println!("  ✓ MCP server registered (user scope) in {}", path.display());
            } else {
                println!("  ✓ MCP server already configured in {}", path.display());
            }
        }
    }

    std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
    Ok(())
}

/// Merge arshy permissions into ~/.claude/settings.json allow list.
fn install_settings_permissions(path: &std::path::Path) -> Result<()> {
    let mut settings: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(obj) = settings.as_object_mut() {
        let perms = obj.entry("permissions").or_insert_with(|| serde_json::json!({}));

        let allow =
            perms.as_object_mut().unwrap().entry("allow").or_insert_with(|| serde_json::json!([]));

        if let Some(arr) = allow.as_array_mut() {
            let existing: Vec<String> =
                arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            let mut added = 0;
            for perm in ARSHY_PERMISSIONS {
                if !existing.contains(&perm.to_string()) {
                    arr.push(serde_json::json!(perm));
                    added += 1;
                }
            }
            if added > 0 {
                println!("  ✓ Added {} permissions to allow list", added);
            } else {
                println!("  ✓ Permissions already configured");
            }
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&settings)?)?;
    Ok(())
}

fn uninstall() -> Result<()> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let claude_dir = home.join(".claude");

    // Remove from ~/.claude.json (top-level mcpServers)
    let claude_json_path = home.join(".claude.json");
    if claude_json_path.exists() {
        let content = std::fs::read_to_string(&claude_json_path)?;
        let mut data: serde_json::Value =
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        if let Some(obj) = data.as_object_mut() {
            if let Some(servers) = obj.get_mut("mcpServers") {
                if let Some(map) = servers.as_object_mut() {
                    map.remove("arshy");
                }
            }
        }
        std::fs::write(&claude_json_path, serde_json::to_string_pretty(&data)?)?;
        println!("Removed arshy from {}", claude_json_path.display());
    }

    // Remove permissions from ~/.claude/settings.json
    let settings_path = claude_dir.join("settings.json");
    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        let mut settings: serde_json::Value =
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        if let Some(obj) = settings.as_object_mut() {
            if let Some(perms) = obj.get_mut("permissions") {
                if let Some(allow) = perms.get_mut("allow") {
                    if let Some(arr) = allow.as_array_mut() {
                        arr.retain(|v| {
                            v.as_str()
                                .map(|s| !s.contains("arshy") && !s.contains("arshyd"))
                                .unwrap_or(true)
                        });
                    }
                }
            }
        }
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
        println!("Removed permissions from {}", settings_path.display());
    }
    Ok(())
}

async fn prune(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    keep: Option<usize>,
    older_than: Option<String>,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let mut params = serde_json::json!({});
    if let Some(k) = keep {
        params["keep"] = serde_json::json!(k);
    }
    if let Some(days) = older_than {
        if let Ok(d) = days.parse::<u64>() {
            params["older_than"] = serde_json::json!(d);
        }
    }
    let request = Request { jsonrpc: "2.0".into(), id: 1, method: METHOD_PRUNE.into(), params };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

fn config(action: ConfigAction, config_path: Option<PathBuf>) -> Result<()> {
    let overrides =
        arshy_lib::config::CliOverrides { config_path: config_path.clone(), ..Default::default() };
    match action {
        ConfigAction::Get { key } => config_get(&key, &overrides),
        ConfigAction::Set { key, value } => {
            config_set(&key, &value, &overrides, config_path.as_deref())
        }
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

/// Set a config value by dot-separated key path, then write to file.
fn config_set(
    key: &str,
    value: &str,
    overrides: &arshy_lib::config::CliOverrides,
    write_path: Option<&std::path::Path>,
) -> Result<()> {
    let mut cfg = Config::load(overrides.clone())?;

    let set_ok = set_config_field(&mut cfg, key, value);

    if !set_ok {
        eprintln!("unknown or read-only key: {}", key);
        eprintln!("settable keys: {}", SETTABLE_KEYS.join(", "));
        std::process::exit(1);
    }

    if let Some(path) = write_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(&cfg)?)?;
    } else {
        cfg.write_to_default_path()?;
    }
    println!("Set {} = {}", key, value);
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
    "store.db_path",
    "store.wal_mode",
    "store.integrity_check",
    "store.auto_prune",
    "store.prune_keep",
    "store.prune_older_than_days",
    "parser.dirs",
    "parser.hot_reload",
    "parser.fallback_to_raw",
    "parser.default_priority",
    "parser.coverage_warning_threshold",
    "parser.version_cache_ttl_hours",
    "notifications.enabled",
    "notifications.batch_interval_ms",
    "notifications.max_batch_events",
    "notifications.min_severity",
    "mcp.client_detection_order",
    "telemetry.enabled",
];

const SETTABLE_KEYS: &[&str] = VALID_KEYS;

fn set_config_field(cfg: &mut Config, key: &str, value: &str) -> bool {
    match key {
        "daemon.socket_path" => {
            cfg.daemon.socket_path = PathBuf::from(value);
        }
        "daemon.log_level" => {
            cfg.daemon.log_level = value.to_string();
        }
        "daemon.log_format" => {
            cfg.daemon.log_format = value.to_string();
        }
        "daemon.auto_start" => {
            cfg.daemon.auto_start = parse_bool(value);
        }
        "daemon.max_task_duration_ms" => {
            if let Ok(v) = value.parse() {
                cfg.daemon.max_task_duration_ms = v;
            } else {
                return false;
            }
        }
        "daemon.max_output_bytes" => {
            if let Ok(v) = value.parse() {
                cfg.daemon.max_output_bytes = v;
            } else {
                return false;
            }
        }
        "daemon.kill_graceful_ms" => {
            if let Ok(v) = value.parse() {
                cfg.daemon.kill_graceful_ms = v;
            } else {
                return false;
            }
        }
        "daemon.kill_force_ms" => {
            if let Ok(v) = value.parse() {
                cfg.daemon.kill_force_ms = v;
            } else {
                return false;
            }
        }
        "daemon.max_concurrent_tasks" => {
            if let Ok(v) = value.parse() {
                cfg.daemon.max_concurrent_tasks = v;
            } else {
                return false;
            }
        }
        "store.db_path" => {
            cfg.store.db_path = PathBuf::from(value);
        }
        "store.wal_mode" => {
            cfg.store.wal_mode = parse_bool(value);
        }
        "store.integrity_check" => {
            cfg.store.integrity_check = parse_bool(value);
        }
        "store.auto_prune" => {
            cfg.store.auto_prune = parse_bool(value);
        }
        "store.prune_keep" => {
            if let Ok(v) = value.parse() {
                cfg.store.prune_keep = v;
            } else {
                return false;
            }
        }
        "store.prune_older_than_days" => {
            if let Ok(v) = value.parse() {
                cfg.store.prune_older_than_days = v;
            } else {
                return false;
            }
        }
        "parser.dirs" => {
            cfg.parser.dirs = value.split(':').map(PathBuf::from).collect();
        }
        "parser.hot_reload" => {
            cfg.parser.hot_reload = parse_bool(value);
        }
        "parser.fallback_to_raw" => {
            cfg.parser.fallback_to_raw = parse_bool(value);
        }
        "parser.default_priority" => {
            if let Ok(v) = value.parse() {
                cfg.parser.default_priority = v;
            } else {
                return false;
            }
        }
        "parser.coverage_warning_threshold" => {
            if let Ok(v) = value.parse() {
                cfg.parser.coverage_warning_threshold = v;
            } else {
                return false;
            }
        }
        "parser.version_cache_ttl_hours" => {
            if let Ok(v) = value.parse() {
                cfg.parser.version_cache_ttl_hours = v;
            } else {
                return false;
            }
        }
        "notifications.enabled" => {
            cfg.notifications.enabled = parse_bool(value);
        }
        "notifications.batch_interval_ms" => {
            if let Ok(v) = value.parse() {
                cfg.notifications.batch_interval_ms = v;
            } else {
                return false;
            }
        }
        "notifications.max_batch_events" => {
            if let Ok(v) = value.parse() {
                cfg.notifications.max_batch_events = v;
            } else {
                return false;
            }
        }
        "notifications.min_severity" => {
            cfg.notifications.min_severity = value.to_string();
        }
        "mcp.client_detection_order" => {
            cfg.mcp.client_detection_order = value.split(',').map(String::from).collect();
        }
        "telemetry.enabled" => {
            cfg.telemetry.enabled = parse_bool(value);
        }
        _ => return false,
    }
    true
}

fn parse_bool(s: &str) -> bool {
    s.eq_ignore_ascii_case("true") || s == "1"
}

async fn daemon_status(config_path: Option<PathBuf>, log_level: Option<String>) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_STATUS.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

async fn daemon_stats(config_path: Option<PathBuf>, log_level: Option<String>) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_STATS.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

async fn daemon_action(
    action: DaemonAction,
    config_path: Option<PathBuf>,
    log_level: Option<String>,
) -> Result<()> {
    match action {
        DaemonAction::Start => daemon_start(config_path).await,
        DaemonAction::Stop => daemon_stop(config_path, log_level).await,
        DaemonAction::Restart => {
            // Stop first (ignore error if not running)
            let _ = daemon_stop(config_path.clone(), log_level.clone()).await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            daemon_start(config_path).await
        }
    }
}

async fn daemon_start(config_path: Option<PathBuf>) -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides { config_path, ..Default::default() })
        .unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();

    if ipc::connect(&socket_path).await.is_ok() {
        println!("daemon is already running");
        return Ok(());
    }

    std::process::Command::new("arshyd")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| arshy_lib::ArshyError::DaemonUnreachable(format!("spawn arshyd: {}", e)))?;

    for _ in 0..25 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if ipc::connect(&socket_path).await.is_ok() {
            println!("daemon started");
            return Ok(());
        }
    }
    Err(arshy_lib::ArshyError::DaemonUnreachable("daemon did not start within 5s".into()))
}

async fn daemon_stop(config_path: Option<PathBuf>, log_level: Option<String>) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_SHUTDOWN.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

/// Install macOS launchd plist for daemon auto-start.
fn install_launchd() -> Result<()> {
    let plist_dir =
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join("Library").join("LaunchAgents");
    let plist_path = plist_dir.join("com.arshy.daemon.plist");

    let arshyd_path = which_arshyd()
        .ok_or_else(|| arshy_lib::ArshyError::Other("arshyd not found in PATH".into()))?;

    let data_dir =
        dirs::data_dir().unwrap_or_else(|| PathBuf::from("~/.local/share")).join("arshy");
    std::fs::create_dir_all(&data_dir)?;

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.arshy.daemon</string>
    <key>ProgramArguments</key>
    <array><string>{}</string></array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>{}/daemon.log</string>
    <key>StandardErrorPath</key><string>{}/daemon.err</string>
</dict>
</plist>"#,
        arshyd_path.display(),
        data_dir.display(),
        data_dir.display(),
    );

    std::fs::create_dir_all(&plist_dir)?;
    std::fs::write(&plist_path, plist_content)?;
    println!("Installed launchd plist to {}", plist_path.display());
    println!("To load:   launchctl load {}", plist_path.display());
    println!("To unload: launchctl unload {}", plist_path.display());
    Ok(())
}

/// Install Linux systemd user unit for daemon auto-start.
fn install_systemd() -> Result<()> {
    let service_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".config")
        .join("systemd")
        .join("user");
    let service_path = service_dir.join("arshyd.service");

    let arshyd_path = which_arshyd()
        .ok_or_else(|| arshy_lib::ArshyError::Other("arshyd not found in PATH".into()))?;

    let service_content = format!(
        r#"[Unit]
Description=Arshy daemon - AI Agent shell execution layer
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
RestartSec=5
StartLimitInterval=120
StartLimitBurst=5
Environment=RUST_BACKTRACE=1

[Install]
WantedBy=default.target
"#,
        arshyd_path.display(),
    );

    std::fs::create_dir_all(&service_dir)?;
    std::fs::write(&service_path, service_content)?;
    println!("Installed systemd unit to {}", service_path.display());
    println!("To enable: systemctl --user enable arshyd.service");
    println!("To start:  systemctl --user start arshyd.service");
    Ok(())
}

/// Find a named binary in PATH, excluding target/ directories (debug builds).
fn find_installed_binary(name: &str) -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let path = PathBuf::from(dir).join(name);
        if path.exists() && !path.to_string_lossy().contains("target") {
            return Some(path);
        }
    }
    None
}

/// Find arshyd binary path.
fn which_arshyd() -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let path = PathBuf::from(dir).join("arshyd");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn doctor(config_path: Option<PathBuf>, log_level: Option<String>) -> Result<()> {
    let mut ok = 0u32;
    let mut fail = 0u32;

    macro_rules! check {
        ($label:expr, $ok:expr, $msg:expr) => {
            if $ok {
                println!("  ✓ {}", $label);
                ok += 1;
            } else {
                println!("  ✗ {} — {}", $label, $msg);
                fail += 1;
            }
        };
    }
    macro_rules! hint {
        ($msg:expr) => {
            println!("    💡 {}", $msg);
        };
    }

    println!("arshy doctor — Claude Code integration diagnostics\n");

    // ── 1. Binary checks ──────────────────────────────────────────────────
    println!("1. Binaries");
    let arshy_path = which_arshy_path();
    check!("arshy in PATH", arshy_path.is_some(), "install with: cargo install arshy");
    let arshyd_path = which_arshyd();
    check!("arshyd in PATH", arshyd_path.is_some(), "install with: cargo install arshy");

    // ── 2. Daemon ─────────────────────────────────────────────────────────
    println!("\n2. Daemon");
    let cfg = arshy_lib::config::Config::load(arshy_lib::config::CliOverrides {
        config_path,
        log_level,
        ..Default::default()
    })
    .unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();
    // Check socket file exists (avoids tokio runtime conflict from within block_on)
    let daemon_running = socket_path.exists();
    check!("daemon is running", daemon_running, "start with: arshy daemon start");
    if !daemon_running {
        hint!("The daemon must be running for MCP calls to work.");
        hint!("Also try: arshy install-launchd  (auto-start on macOS)");
    }

    // ── 3. MCP server config ──────────────────────────────────────────────
    println!("\n3. MCP server (~/.claude.json)");
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let claude_dir = home.join(".claude");
    let claude_json_path = home.join(".claude.json");
    let mcp_ok = check_claude_json_mcp(&claude_json_path);
    check!("arshy entry in ~/.claude.json", mcp_ok, "run: arshy install");
    if !mcp_ok {
        hint!("The MCP server must be registered for Claude Code to see arshy tools.");
    }

    // ── 4. Permissions ────────────────────────────────────────────────────
    println!("\n4. Permissions (settings.json)");
    let settings_path = claude_dir.join("settings.json");
    let perm_status = check_permissions(&settings_path);
    match &perm_status {
        PermStatus::AllGood => {
            println!("  ✓ all arshy permissions configured");
            ok += 1;
        }
        PermStatus::Missing(missing) => {
            println!("  ✗ missing {} permission(s):", missing.len());
            for m in missing {
                println!("    - {}", m);
            }
            fail += 1;
            hint!("run: arshy install  (auto-adds missing permissions)");
        }
        PermStatus::FileMissing => {
            println!("  ✗ settings.json not found — run: arshy install");
            fail += 1;
        }
    }

    // ── 5. Summary ────────────────────────────────────────────────────────
    println!("\n{}", "─".repeat(50));
    println!("  {} passed, 0 warnings, {} failed", ok, fail);
    if fail == 0 {
        println!("\n  🎉 Everything looks good! Restart Claude Code to activate arshy.");
    } else {
        println!("\n  Run `arshy install` to fix most issues automatically.");
    }

    Ok(())
}

/// Find arshy binary path.
fn which_arshy_path() -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let path = PathBuf::from(dir).join("arshy");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn check_claude_json_mcp(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let data: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    // Check top-level mcpServers (user scope)
    data.get("mcpServers").and_then(|s| s.get("arshy")).is_some()
}

enum PermStatus {
    AllGood,
    Missing(Vec<String>),
    FileMissing,
}

fn check_permissions(path: &std::path::Path) -> PermStatus {
    if !path.exists() {
        return PermStatus::FileMissing;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return PermStatus::FileMissing,
    };
    let settings: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return PermStatus::FileMissing,
    };

    let existing: Vec<String> = settings
        .get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(|a| a.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let missing: Vec<String> = ARSHY_PERMISSIONS
        .iter()
        .filter(|p| !existing.contains(&p.to_string()))
        .map(|p| p.to_string())
        .collect();

    if missing.is_empty() {
        PermStatus::AllGood
    } else {
        PermStatus::Missing(missing)
    }
}

// ── Parser management ──────────────────────────────────────────────────────

async fn parser_action(
    action: ParserAction,
    config_path: Option<PathBuf>,
    log_level: Option<String>,
) -> Result<()> {
    match action {
        ParserAction::Reload => parser_reload(config_path, log_level).await,
        ParserAction::List => parser_list(config_path, log_level).await,
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
        .args(["test", "--bin", "arshyd", "benchmark::run_benchmark", "--", "--nocapture"])
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
    let result: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| arshy_lib::ArshyError::Other(format!("invalid benchmark JSON: {}", e)))?;

    render::render_benchmark(&result);

    Ok(())
}
