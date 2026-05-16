//! CLI command dispatch — connects to daemon via UDS and executes user commands.

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, QueryParams, Request, RunTaskParams, METHOD_LIST, METHOD_PRUNE, METHOD_RUN, METHOD_SHUTDOWN, METHOD_STATS, METHOD_STATUS};
use arshy_lib::Result;
use std::path::PathBuf;

use crate::{Cli, CliCommand, ConfigAction, DaemonAction};

/// Route a parsed CLI to the appropriate handler.
pub async fn dispatch(cli: Cli) -> Result<()> {
    let config_path = cli.config;
    let log_level = cli.log_level;

    match cli.command {
        Some(CliCommand::Run { command, cwd, timeout_ms, mode }) => {
            run_command(config_path, log_level, &command, cwd, timeout_ms, mode).await
        }
        Some(CliCommand::List { status, limit }) => {
            list_tasks(config_path, log_level, status, limit).await
        }
        Some(CliCommand::Query { task_id, event_type, severity, code, file, limit }) => {
            query_events(config_path, log_level, task_id, event_type, severity, code, file, limit).await
        }
        Some(CliCommand::Kill { task_id }) => {
            kill_task(config_path, log_level, &task_id).await
        }
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
        None => {
            println!("Arshy — AI Agent native shell execution layer");
            println!("Usage: arshy [--from-mcp] [OPTIONS] <COMMAND>");
            Ok(())
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

async fn connect(config_path: Option<PathBuf>, _log_level: Option<String>) -> Result<tokio::net::UnixStream> {
    let cfg = Config::load(arshy_lib::config::CliOverrides {
        config_path,
        ..Default::default()
    })
    .unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();
    ipc::connect(&socket_path).await
}

// ── Subcommand handlers ────────────────────────────────────────────────────────

async fn run_command(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    command: &str,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    mode: Option<String>,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let params = RunTaskParams {
        command: command.into(),
        cwd,
        timeout_ms,
        mode: mode.unwrap_or_else(|| "sync".into()),
        parse_hint: None,
    };
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_RUN.into(),
        params: serde_json::to_value(&params)?,
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
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
    let params = QueryParams {
        task_id,
        event_type,
        severity,
        code,
        file,
        limit,
        offset: 0,
    };
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

fn install() -> Result<()> {
    println!("Registering arshy as MCP server...");
    let settings_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".claude")
        .join("settings.json");

    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let arshy_entry = serde_json::json!({
        "command": "arshy",
        "args": ["--from-mcp"],
        "type": "stdio"
    });

    if let Some(obj) = settings.as_object_mut() {
        let mcp_servers = obj
            .entry("mcpServers")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(servers) = mcp_servers.as_object_mut() {
            servers.insert("arshy".into(), arshy_entry);
        }

        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
        println!("Installed to {}", settings_path.display());
    }

    Ok(())
}

fn uninstall() -> Result<()> {
    let settings_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".claude")
        .join("settings.json");

    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        let mut settings: serde_json::Value =
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        if let Some(obj) = settings.as_object_mut() {
            if let Some(servers) = obj.get_mut("mcpServers") {
                if let Some(map) = servers.as_object_mut() {
                    map.remove("arshy");
                }
            }
        }
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
        println!("Removed arshy from {}", settings_path.display());
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
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_PRUNE.into(),
        params,
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    println!("{}", serde_json::to_string_pretty(&response.result)?);
    Ok(())
}

fn config(action: ConfigAction, config_path: Option<PathBuf>) -> Result<()> {
    let overrides = arshy_lib::config::CliOverrides {
        config_path: config_path.clone(),
        ..Default::default()
    };
    match action {
        ConfigAction::Get { key } => config_get(&key, &overrides),
        ConfigAction::Set { key, value } => config_set(&key, &value, &overrides, config_path.as_deref()),
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
fn config_set(key: &str, value: &str, overrides: &arshy_lib::config::CliOverrides, write_path: Option<&std::path::Path>) -> Result<()> {
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
    "daemon.socket_path", "daemon.log_level", "daemon.log_format",
    "daemon.auto_start", "daemon.max_task_duration_ms", "daemon.max_output_bytes",
    "daemon.kill_graceful_ms", "daemon.kill_force_ms", "daemon.max_concurrent_tasks",
    "store.db_path", "store.wal_mode", "store.integrity_check",
    "store.auto_prune", "store.prune_keep", "store.prune_older_than_days",
    "parser.dirs", "parser.hot_reload", "parser.fallback_to_raw",
    "parser.default_priority", "parser.coverage_warning_threshold",
    "parser.version_cache_ttl_hours",
    "notifications.enabled", "notifications.batch_interval_ms",
    "notifications.max_batch_events", "notifications.min_severity",
    "mcp.client_detection_order",
    "telemetry.enabled",
];

const SETTABLE_KEYS: &[&str] = VALID_KEYS;

fn set_config_field(cfg: &mut Config, key: &str, value: &str) -> bool {
    match key {
        "daemon.socket_path" => { cfg.daemon.socket_path = PathBuf::from(value); }
        "daemon.log_level" => { cfg.daemon.log_level = value.to_string(); }
        "daemon.log_format" => { cfg.daemon.log_format = value.to_string(); }
        "daemon.auto_start" => { cfg.daemon.auto_start = parse_bool(value); }
        "daemon.max_task_duration_ms" => {
            if let Ok(v) = value.parse() { cfg.daemon.max_task_duration_ms = v; }
            else { return false; }
        }
        "daemon.max_output_bytes" => {
            if let Ok(v) = value.parse() { cfg.daemon.max_output_bytes = v; }
            else { return false; }
        }
        "daemon.kill_graceful_ms" => {
            if let Ok(v) = value.parse() { cfg.daemon.kill_graceful_ms = v; }
            else { return false; }
        }
        "daemon.kill_force_ms" => {
            if let Ok(v) = value.parse() { cfg.daemon.kill_force_ms = v; }
            else { return false; }
        }
        "daemon.max_concurrent_tasks" => {
            if let Ok(v) = value.parse() { cfg.daemon.max_concurrent_tasks = v; }
            else { return false; }
        }
        "store.db_path" => { cfg.store.db_path = PathBuf::from(value); }
        "store.wal_mode" => { cfg.store.wal_mode = parse_bool(value); }
        "store.integrity_check" => { cfg.store.integrity_check = parse_bool(value); }
        "store.auto_prune" => { cfg.store.auto_prune = parse_bool(value); }
        "store.prune_keep" => {
            if let Ok(v) = value.parse() { cfg.store.prune_keep = v; }
            else { return false; }
        }
        "store.prune_older_than_days" => {
            if let Ok(v) = value.parse() { cfg.store.prune_older_than_days = v; }
            else { return false; }
        }
        "parser.dirs" => {
            cfg.parser.dirs = value.split(':').map(PathBuf::from).collect();
        }
        "parser.hot_reload" => { cfg.parser.hot_reload = parse_bool(value); }
        "parser.fallback_to_raw" => { cfg.parser.fallback_to_raw = parse_bool(value); }
        "parser.default_priority" => {
            if let Ok(v) = value.parse() { cfg.parser.default_priority = v; }
            else { return false; }
        }
        "parser.coverage_warning_threshold" => {
            if let Ok(v) = value.parse() { cfg.parser.coverage_warning_threshold = v; }
            else { return false; }
        }
        "parser.version_cache_ttl_hours" => {
            if let Ok(v) = value.parse() { cfg.parser.version_cache_ttl_hours = v; }
            else { return false; }
        }
        "notifications.enabled" => { cfg.notifications.enabled = parse_bool(value); }
        "notifications.batch_interval_ms" => {
            if let Ok(v) = value.parse() { cfg.notifications.batch_interval_ms = v; }
            else { return false; }
        }
        "notifications.max_batch_events" => {
            if let Ok(v) = value.parse() { cfg.notifications.max_batch_events = v; }
            else { return false; }
        }
        "notifications.min_severity" => { cfg.notifications.min_severity = value.to_string(); }
        "mcp.client_detection_order" => {
            cfg.mcp.client_detection_order = value.split(',').map(String::from).collect();
        }
        "telemetry.enabled" => { cfg.telemetry.enabled = parse_bool(value); }
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
    let cfg = Config::load(arshy_lib::config::CliOverrides {
        config_path,
        ..Default::default()
    }).unwrap_or_default();
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
    let plist_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join("Library")
        .join("LaunchAgents");
    let plist_path = plist_dir.join("com.arshy.daemon.plist");

    let arshyd_path = which_arshyd()
        .ok_or_else(|| arshy_lib::ArshyError::Other("arshyd not found in PATH".into()))?;

    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("arshy");
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
