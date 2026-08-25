//! Task subcommand handlers — run / list / query / kill / tail / prune.

use arshy_lib::config::Config;
use arshy_lib::ipc::{
    self, QueryParams, Request, RunTaskParams, METHOD_LIST, METHOD_PRUNE, METHOD_RUN,
};
use arshy_lib::Result;
use std::path::PathBuf;

use super::render;

// ── Helpers ────────────────────────────────────────────────────────────────────

pub(crate) async fn connect(
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
pub(crate) async fn run_command(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    command: &str,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    mode: Option<String>,
    format: &str,
    errors_only: bool,
    purpose: Option<String>,
) -> Result<()> {
    // Auto-start the daemon on demand, like the MCP proxy and bash wrapper —
    // a bare `arshy run "cmd"` on a fresh install must never fail with a
    // cryptic "No such file" socket error.
    let cfg = Config::load(arshy_lib::config::CliOverrides {
        config_path,
        log_level,
        ..Default::default()
    })
    .unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();
    let mut daemon = crate::proxy::connect_or_start(&cfg, &socket_path).await?;
    // Default cwd to the CLI's current directory. Without this, the daemon
    // records cwd=None, and project context would otherwise fall back to the
    // daemon's own cwd instead of the user's command directory.
    let cwd = match cwd {
        Some(value) if std::path::Path::new(&value).is_relative() => {
            std::env::current_dir().ok().map(|base| base.join(value).to_string_lossy().into_owned())
        }
        Some(value) => Some(value),
        None => std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()),
    };
    let params = RunTaskParams {
        command: command.into(),
        cwd,
        timeout_ms,
        mode: mode.unwrap_or_else(|| "auto".into()),
        parse_hint: None,
        env: None,
        errors_only,
        purpose,
        dedup_key: None,
    };
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_RUN.into(),
        params: serde_json::to_value(&params)?,
    };
    let response = ipc::send_request(&mut daemon, &request).await?;

    match format {
        "pretty" => {
            // Explicit human observation via `arshy run "cmd" --format pretty`.
            eprint!("{}", render::render_run_text(&response.result));
        }
        "json" => {
            println!("{}", serde_json::to_string_pretty(&response.result)?);
        }
        _ => {
            // auto: humans get the concise text when stdout is a terminal,
            // machines get JSON (agent-first shell).
            use std::io::IsTerminal;
            if std::io::stdout().is_terminal() {
                eprint!("{}", render::render_run_text(&response.result));
            } else {
                println!("{}", serde_json::to_string_pretty(&response.result)?);
            }
        }
    }
    let exit_code = response.result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
    std::process::exit(exit_code as i32);
}

pub(crate) async fn list_tasks(
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
pub(crate) async fn query_events(
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
        task_id: Some(task_id),
        event_type,
        severity,
        code,
        file,
        limit,
        offset: 0,
        include_logs: false,
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

pub(crate) async fn kill_task(
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

pub(crate) async fn tail_task(
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

pub(crate) async fn prune(
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
