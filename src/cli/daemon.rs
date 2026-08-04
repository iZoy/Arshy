//! Daemon management and doctor diagnostics.

use arshy_lib::config::Config;
use arshy_lib::ipc::{self, Request, METHOD_SHUTDOWN, METHOD_STATS, METHOD_STATUS};
use arshy_lib::Result;
use std::path::PathBuf;

use super::install::ARSHY_PERMISSIONS;
use super::integrate;
use super::render;
use super::tasks::connect;
use crate::DaemonAction;

pub(crate) async fn daemon_status(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
) -> Result<()> {
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

pub(crate) async fn daemon_stats(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    format: &str,
) -> Result<()> {
    let mut daemon = connect(config_path, log_level).await?;
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_STATS.into(),
        params: serde_json::json!({}),
    };
    let response = ipc::send_request(&mut daemon, &request).await?;
    match format {
        // Raw JSON for automation / evidence snapshots (agent consumption).
        "json" => println!("{}", serde_json::to_string_pretty(&response.result)?),
        _ => eprint!("{}", render::render_stats(&response.result)),
    }
    Ok(())
}

pub(crate) async fn daemon_action(
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

    // Spawn via the shared detached-spawn entry (sibling arshyd + setsid +
    // spawn-lock + circuit breaker), identical to the MCP proxy's auto-start.
    // A bare `Command::spawn` here would leave the daemon in this process's
    // group: it dies when a short-lived caller session (agent tool call, CI
    // step, script) is torn down, making `daemon start` silently useless.
    crate::proxy::start_daemon()?;

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

/// Find a named binary in PATH, excluding target/ directories (debug builds).
fn which_arshyd() -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let path = PathBuf::from(dir).join("arshyd");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

pub(crate) fn doctor(
    config_path: Option<PathBuf>,
    log_level: Option<String>,
    agent: Option<&str>,
) -> Result<()> {
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

    // Reject unknown agent ids up front (same contract as `uninstall --agent`),
    // instead of silently skipping every per-agent check.
    if let Some(id) = agent {
        let valid: Vec<&str> = integrate::all_agents().iter().map(|a| a.id()).collect();
        if !valid.contains(&id) {
            return Err(arshy_lib::ArshyError::Other(format!(
                "unknown agent `{id}`; valid ids: {}",
                valid.join(", ")
            )));
        }
    }

    println!("arshy doctor — IDE integration diagnostics\n");

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
        hint!(
            "It auto-starts on demand with the first command; no OS-level registration is needed."
        );
    }

    // Stale spawn-lock recovery: a crashed proxy may leave /tmp/arshyd.spawn-lock
    // behind, which makes future auto-start believe a spawn is already in flight
    // and silently no-op. If the lock exists but the daemon is clearly down,
    // clear it and tell the user how to bring the daemon back.
    let spawn_lock = std::path::PathBuf::from("/tmp/arshyd.spawn-lock");
    if spawn_lock.exists() && !daemon_running {
        check!(
            "no stale spawn-lock",
            false,
            "cleared /tmp/arshyd.spawn-lock (left behind by a crashed proxy)"
        );
        hint!("A stale spawn-lock was blocking auto-start. It has been removed.");
        hint!("Restart the daemon: arshy daemon restart");
        let _ = std::fs::remove_file(&spawn_lock);
    }

    // ── 3. MCP server config ──────────────────────────────────────────────
    println!("\n3. MCP server config");
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let claude_dir = home.join(".claude");
    let claude_json_path = home.join(".claude.json");
    let mcp_ok = check_claude_json_mcp(&claude_json_path);
    check!("arshy entry in ~/.claude.json (Claude Code)", mcp_ok, "run: arshy install");
    if !mcp_ok {
        hint!("The MCP server must be registered for Claude Code to see arshy tools.");
    }
    let cursor_mcp_path = home.join(".cursor").join("mcp.json");
    let cursor_ok = check_claude_json_mcp(&cursor_mcp_path);
    check!("arshy entry in ~/.cursor/mcp.json (Cursor)", cursor_ok, "run: arshy install");
    if !cursor_ok {
        hint!("Run arshy install to register with Cursor IDE.");
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

    // ── 5. Filesystem access (macOS TCC) ───────────────────────────────
    println!("\n5. Filesystem access");
    let restricted = probe_common_tcc_dirs();
    if restricted.is_empty() {
        println!("  ✓ all common directories accessible");
        ok += 1;
    } else {
        for dir in &restricted {
            println!("  ✗ {} — restricted by macOS TCC", dir.display());
            fail += 1;
        }
        hint!("arshy auto-creates /tmp symlinks so commands still work, but some");
        hint!("tools may see /tmp paths instead of the real project directory.");
        hint!("To fix permanently: System Settings → Privacy & Security →");
        hint!("Full Disk Access → add your terminal app (Terminal.app / iTerm2 / etc.)");
    }

    // ── 5.5 Agent shell interception (dogfooding readiness) ──────────────
    println!("\n5.5 Agent shell interception (dogfooding)");
    let hook_active = crate::cli::shell_wrapper::is_hook_active();
    check!(
        "shell hook shims active in PATH",
        hook_active,
        "run: arshy hook install  (must shadow /bin/bash with the arshy shim)"
    );
    let cwd = std::env::current_dir().unwrap_or_default();
    let ws_configured = crate::cli::shell_wrapper::is_workspace_configured(&cwd);
    check!(
        "current workspace opted in (.arshy.toml / .arshy/)",
        ws_configured,
        "run: arshy init  (in the workspace you want arshy to intercept)"
    );
    let auto_start = cfg.daemon.auto_start;
    check!(
        "daemon auto-start enabled",
        auto_start,
        "set daemon.auto_start = true (ARSHY_DAEMON_AUTO_START=false disables it)"
    );
    let will_intercept = hook_active && ws_configured && auto_start;
    if will_intercept {
        println!("  ✓ agent `bash -c` in THIS workspace will be intercepted and auto-start arshyd");
    } else {
        println!(
            "  ✗ agent bash in THIS workspace will NOT reach arshy — dogfooding is inactive here"
        );
        hint!("Fix the failing checks above, then re-run this from the workspace root.");
        hint!("Also ensure the agent invokes `bash` (PATH-resolved), not /bin/bash, which");
        hint!("bypasses the shim entirely.");
    }

    // ── 5.6 Per-agent integration (omni-integration) ───────────────────
    println!("\n5.6 Agent integrations (arshy across environments)");
    for s in integrate::agent_statuses() {
        if let Some(id) = agent {
            if s.id != id {
                continue;
            }
        }
        if !s.detected {
            println!("  · {} — not detected", s.name);
            continue;
        }
        if s.active {
            println!("  ✓ {} ({}) — {}", s.name, s.mechanism, s.detail);
            ok += 1;
        } else {
            println!("  ✗ {} ({}) — {}", s.name, s.mechanism, s.detail);
            fail += 1;
        }
    }

    // ── 6. Summary ────────────────────────────────────────────────────────
    println!("\n{}", "─".repeat(50));
    println!("  {} passed, 0 warnings, {} failed", ok, fail);
    if fail == 0 {
        println!("\n  Everything looks good! Restart your IDE to activate arshy.");
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

/// Probe common macOS TCC-restricted directories for accessibility.
/// Returns a list of directories that are likely restricted by TCC.
fn probe_common_tcc_dirs() -> Vec<PathBuf> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return vec![],
    };
    let candidates = [home.join("Documents"), home.join("Desktop"), home.join("Downloads")];
    candidates
        .into_iter()
        .filter(|dir| {
            // Only flag directories that actually exist AND are likely TCC-restricted.
            // TCC restricts child processes (PTY) from accessing these top-level dirs,
            // even when the parent process (daemon) can write there.
            dir.exists()
        })
        .collect()
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
