//! CLI command dispatch — connects to daemon via UDS and executes user commands.

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, QueryParams, Request, RunTaskParams, METHOD_LIST, METHOD_PRUNE, METHOD_RUN, METHOD_STATUS};
use arshy_lib::Result;
use std::path::PathBuf;

use crate::{Cli, CliCommand, ConfigAction};

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
        Some(CliCommand::Config { action }) => config(action),
        Some(CliCommand::Status) => daemon_status(config_path, log_level).await,
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

fn config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Get { key } => {
            let cfg = Config::load(arshy_lib::config::CliOverrides::default())?;
            let value = match key.as_str() {
                "daemon.log_level" => serde_json::json!(cfg.daemon.log_level),
                "daemon.auto_start" => serde_json::json!(cfg.daemon.auto_start),
                "daemon.max_concurrent_tasks" => serde_json::json!(cfg.daemon.max_concurrent_tasks),
                "store.wal_mode" => serde_json::json!(cfg.store.wal_mode),
                "parser.hot_reload" => serde_json::json!(cfg.parser.hot_reload),
                "notifications.enabled" => serde_json::json!(cfg.notifications.enabled),
                _ => serde_json::json!(format!("unknown key: {}", key)),
            };
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        ConfigAction::Set { key, value } => {
            let cfg = Config::load(arshy_lib::config::CliOverrides::default())?;
            match key.as_str() {
                "daemon.log_level" => {
                    let mut c = cfg;
                    c.daemon.log_level = value;
                    c.write_to_default_path()?;
                    println!("Set daemon.log_level = {}", c.daemon.log_level);
                }
                _ => println!("Setting '{}' is not yet supported in this version", key),
            }
        }
        ConfigAction::List => {
            let cfg = Config::load(arshy_lib::config::CliOverrides::default())?;
            let serialized = toml::to_string_pretty(&cfg)?;
            println!("{}", serialized);
        }
    }
    Ok(())
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
