use arshy_lib::config::Config;
use arshy_lib::ipc::{self, Request, RunTaskParams, METHOD_LIST, METHOD_QUERY, METHOD_RUN};
use arshy_lib::Result;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Environment variable that prevents recursive interception when set to "1".
pub const ARSHY_BYPASS: &str = "ARSHY_BYPASS";

/// Global kill switch (decision 3): set `ARSHY_NO_INTERCEPT=1` to disable
/// ALL bash interception on this machine — every `bash -c` passes through
/// transparently. Stronger than the per-command `ARSHY_BYPASS`.
pub const ARSHY_NO_INTERCEPT: &str = "ARSHY_NO_INTERCEPT";

/// Main entry point for shell wrapper interception.
/// Checks if it should intercept the shell command or transparently pass through.
pub fn run_wrapper(exe_name: &str, args: Vec<String>) -> Result<()> {
    // Special case: if ARSHY_BYPASS is set but we're not the main arshy binary,
    // this means we're being called as a subprocess from a hook that already set the bypass.
    // In this case, skip all interception logic and just execute the real command.
    // This prevents re-interception when claude-hook rewrites to `ARSHY_BYPASS=1 arshy run ...`
    if std::env::var(ARSHY_BYPASS).is_ok() && exe_name != "arshy" && exe_name != "arshyd" {
        return execute_real_shell(exe_name, &args);
    }

    // Global kill switch (decision 3): ARSHY_NO_INTERCEPT=1 disables all
    // interception; every `bash -c` passes through transparently.
    if std::env::var(ARSHY_NO_INTERCEPT).is_ok() && exe_name != "arshy" && exe_name != "arshyd" {
        return execute_real_shell(exe_name, &args);
    }

    // Only `bash -c "cmd"` is interceptable. Interactive shells pass through
    // with no decision cost and no audit noise.
    let mut command_to_run = None;
    for (i, arg) in args.iter().enumerate() {
        if arg == "-c" {
            if let Some(cmd) = args.get(i + 1) {
                command_to_run = Some(cmd.clone());
            }
            break;
        }
    }
    let Some(command) = command_to_run else {
        return execute_real_shell(exe_name, &args);
    };

    // Only intercept (and thus auto-start the daemon) when the current workspace has
    // opted in via a `.arshy.toml` marker or `.arshy/` directory.
    let workspace_configured =
        std::env::current_dir().map(|d| is_workspace_configured(&d)).unwrap_or(false);

    // Cheap, O(1) env check first. Only when no agent env marker is present do we
    // pay the cost of resolving the parent process name (which forks `ps`). This
    // keeps the per-command overhead low for ordinary non-agent shells.
    let is_agent_env = std::env::var("ARSHY_AGENT").is_ok()
        || std::env::var("CLAUDE_CODE").is_ok()
        || std::env::var("CLINE_AGENT").is_ok();
    let parent_name = if is_agent_env { None } else { get_parent_process_name() };

    let decision = should_intercept(
        &args,
        |key| std::env::var(key).is_ok(),
        &parent_name,
        workspace_configured,
    );

    // Audit the decision (decision 3): agent-relevant evaluations are appended
    // to ~/.arshy/intercept.jsonl — who, what, why. Pass-throughs in ordinary
    // human shells are skipped to avoid noise.
    if decision.intercept || is_agent_env || workspace_configured {
        audit_intercept_decision(
            exe_name,
            &command,
            parent_name.as_deref(),
            workspace_configured,
            &decision,
        );
    }

    if decision.intercept {
        // Run synchronously via Tokio block_on (we are in main)
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        rt.block_on(async {
            if let Err(e) = intercept_and_run(exe_name, &args, command).await {
                // If interception fails, fallback to real shell
                eprintln!("Arshy interception failed: {}. Falling back to real shell...", e);
                let _ = execute_real_shell(exe_name, &args);
            }
        });
        Ok(())
    } else {
        execute_real_shell(exe_name, &args)
    }
}

/// Append one interception decision to `~/.arshy/intercept.jsonl`
/// (best-effort — audit failures never affect command execution).
fn audit_intercept_decision(
    exe_name: &str,
    command: &str,
    parent: Option<&str>,
    workspace_configured: bool,
    decision: &InterceptDecision,
) {
    let Some(home) = dirs::home_dir() else { return };
    let dir = home.join(".arshy");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let line = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "exe": exe_name,
        "command": command,
        "parent": parent,
        "workspace_configured": workspace_configured,
        "intercept": decision.intercept,
        "reason": decision.reason,
    });
    let Ok(mut file) =
        std::fs::OpenOptions::new().create(true).append(true).open(dir.join("intercept.jsonl"))
    else {
        return;
    };
    let _ = writeln!(file, "{}", line);
}

/// A single interception decision: whether to intercept plus a stable reason
/// for audit / doctor explainability.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterceptDecision {
    pub intercept: bool,
    pub reason: &'static str,
}

/// Decides whether to intercept the shell command based on command arguments,
/// environment variables, parent process name, and whether the current workspace
/// has opted in. Also detects potential TTY conflicts and recursive calls.
///
/// Returns a reason for every outcome (decision 3) so the decision can be
/// audited — a pass-through with a reason is as diagnosable as a catch.
fn should_intercept(
    args: &[String],
    env_check: impl Fn(&str) -> bool,
    parent_name: &Option<String>, // Changed from owned to borrowed
    workspace_configured: bool,
) -> InterceptDecision {
    let mut has_c = false;
    for arg in args {
        if arg == "-c" {
            has_c = true;
            break;
        }
    }
    if !has_c {
        return InterceptDecision { intercept: false, reason: "not a -c command" };
    }

    let is_agent_env =
        env_check("ARSHY_AGENT") || env_check("CLAUDE_CODE") || env_check("CLINE_AGENT");

    // Check if parent process directly controls TTY — these should bypass to avoid UI conflict
    // Use ref to avoid moving parent_name so we can use it again later for is_agent_parent check
    let has_tty_conflict = if let Some(ref parent) = parent_name {
        let name = parent.to_lowercase();
        name.contains("vim")
            || name.contains("nvim")
            || name.contains("less")
            || name.contains("more")
            || name.contains("htop")
            || name.contains("tmux")
            || name.contains("screen")
    } else {
        false
    };

    // Check for recursive interception: if args contain "arshy" or "claude-hook",
    // we're likely already within an intercepted command chain
    let arg_contains_arshy = args.iter().any(|a| a.contains("arshy") || a.contains("claude-hook"));

    // Agent parents split into two buckets:
    // - Unambiguous desktop agents (codex, claude, cursor, ...): intercept even
    //   without a workspace opt-in — this is what makes the proxy take over
    //   "almost all bash" for agents with zero per-project setup.
    // - Ambiguous parents (generic "agent", node, python): still require the
    //   workspace marker so scripts/build tools never surprise the user.
    // Whitelist stays aligned with the 8 first-class agents in
    // `src/cli/integrate.rs` (claude-code, cursor, vscode, antigravity, codex,
    // opencode, aider, workbuddy) plus their process-name variants. Other MCP
    // agents (Copilot CLI, Cline, Roo, ...) are NOT passively intercepted —
    // they opt in explicitly via the manual MCP guide (docs/how-to/
    // manual-mcp.md) or a workspace marker.
    let is_known_agent_parent = if let Some(ref parent) = parent_name {
        let name = parent.to_lowercase();
        [
            "codex",
            "claude",
            "cursor",
            "vscode",
            "code helper",
            "gemini",
            "antigravity",
            "agy",
            "opencode",
            "aider",
            "workbuddy",
        ]
        .iter()
        .any(|k| name.contains(k))
    } else {
        false
    };

    let is_ambiguous_agent_parent = if let Some(ref parent) = parent_name {
        let name = parent.to_lowercase();
        name.contains("agent") || name.contains("node") || name.contains("python")
    } else {
        false
    };

    // Only intercept when:
    // 1. Workspace opted in, OR the spawner is an unambiguous desktop agent
    // 2. Not in a TTY conflict situation
    // 3. Not a recursive call (prevents double-interception from claude-hook rewrite)
    if !workspace_configured && !is_known_agent_parent {
        return InterceptDecision {
            intercept: false,
            reason: "no workspace opt-in and not a known agent parent",
        };
    }

    if has_tty_conflict {
        return InterceptDecision { intercept: false, reason: "tty conflict (vim/less/tmux...)" };
    }

    if arg_contains_arshy {
        return InterceptDecision {
            intercept: false,
            reason: "recursive call (arshy/claude-hook in args)",
        };
    }

    if is_agent_env {
        InterceptDecision { intercept: true, reason: "agent env marker" }
    } else if is_known_agent_parent {
        InterceptDecision { intercept: true, reason: "known agent parent" }
    } else if is_ambiguous_agent_parent {
        InterceptDecision {
            intercept: true,
            reason: "ambiguous agent parent with workspace opt-in",
        }
    } else {
        InterceptDecision { intercept: false, reason: "workspace opted in but no agent signal" }
    }
}

/// Returns true if `start` (or any ancestor directory up to the filesystem root)
/// contains an arshy workspace marker: a `.arshy.toml` file or an `.arshy/` directory.
pub(crate) fn is_workspace_configured(start: &std::path::Path) -> bool {
    let mut current = Some(start);
    while let Some(dir) = current {
        if dir.join(".arshy.toml").is_file() || dir.join(".arshy").is_dir() {
            return true;
        }
        current = dir.parent();
    }
    false
}

async fn intercept_and_run(_exe_name: &str, _args: &[String], command: String) -> Result<()> {
    let cfg = Config::load(arshy_lib::config::CliOverrides::default()).unwrap_or_default();
    let socket_path = cfg.daemon.expanded_socket_path();

    // Connect to daemon, auto-starting it on demand when `daemon.auto_start` is set
    // (default true) and the daemon is not yet running. This is what makes a bare
    // `bash -c` from an agent bring up arshyd transparently.
    let mut daemon = crate::proxy::connect_or_start(&cfg, &socket_path).await?;

    let params = RunTaskParams {
        command,
        cwd: None,
        timeout_ms: None,
        mode: "auto".into(),
        parse_hint: None,
        env: None,
        errors_only: false,
        purpose: None,
        dedup_key: None,
    };
    let request = Request {
        jsonrpc: "2.0".into(),
        id: 1,
        method: METHOD_RUN.into(),
        params: serde_json::to_value(&params)?,
    };

    let response = ipc::send_request(&mut daemon, &request).await?;

    if response.result.get("error").is_some() {
        let err_msg = response.result["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(arshy_lib::ArshyError::DaemonUnreachable(err_msg.to_string()));
    }

    // Auto mode degrades to async when a long task exceeds the sync patience
    // (60s). Follow the task to a terminal state (bounded), then report — the
    // daemon keeps executing even if this wrapper is killed, so this is how
    // long tasks get delegated through the bash proxy.
    if response.result["status"].as_str() == Some("running") {
        let task_id = response.result["task_id"].as_str().unwrap_or("").to_string();
        follow_task(&mut daemon, &task_id).await?;
        return Ok(());
    }

    // Agent-first: pass through structured text — raw output for short
    // commands, a concise summary + top events for long commands.
    print!("{}", super::render::render_run_text(&response.result));
    let _ = std::io::stdout().flush();

    let exit_code = response.result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
    std::process::exit(exit_code as i32);
}

/// Poll a delegated task (auto mode degraded to async) until it reaches a
/// terminal state, then print the final summary. Bounded patience: if the task
/// is still running after the cap, print a delegation hint and exit 0 — the
/// daemon keeps executing, and the agent can poll or kill via `arshy`.
async fn follow_task(daemon: &mut tokio::net::UnixStream, task_id: &str) -> Result<()> {
    use std::time::{Duration, Instant};

    let poll_interval = Duration::from_secs(2);
    let patience = Duration::from_secs(120);
    let start = Instant::now();

    loop {
        let request = Request {
            jsonrpc: "2.0".into(),
            id: 2,
            method: METHOD_LIST.into(),
            params: serde_json::json!({ "limit": 1000 }),
        };
        match ipc::send_request(daemon, &request).await {
            Ok(response) => {
                if let Some(task) = response.result.as_array().and_then(|tasks| {
                    tasks.iter().find(|t| t["task_id"].as_str() == Some(task_id)).cloned()
                }) {
                    let status = task["status"].as_str().unwrap_or("running");
                    if status != "running" {
                        // Terminal: print final summary + top error events.
                        let icon = match status {
                            "completed" => "✓",
                            "failed" | "timeout" => "✗",
                            "killed" => "⊘",
                            _ => "?",
                        };
                        let mut out = format!(
                            "{} task {}: {}{}",
                            icon,
                            task_id,
                            status,
                            task["exit_code"]
                                .as_i64()
                                .map(|c| format!(" (exit {})", c))
                                .unwrap_or_default()
                        );
                        if let Some(d) = task["duration_ms"].as_u64() {
                            out.push_str(&format!(", {:.1}s", d as f64 / 1000.0));
                        }
                        if let Some(ec) = task["error_count"].as_u64() {
                            if ec > 0 {
                                out.push_str(&format!(
                                    ", {} error{}",
                                    ec,
                                    if ec == 1 { "" } else { "s" }
                                ));
                            }
                        }
                        out.push('\n');
                        let qreq = Request {
                            jsonrpc: "2.0".into(),
                            id: 3,
                            method: METHOD_QUERY.into(),
                            params: serde_json::json!({
                                "task_id": task_id,
                                "severity": "error",
                                "limit": 8
                            }),
                        };
                        if let Ok(qresp) = ipc::send_request(daemon, &qreq).await {
                            let events = qresp.result.get("events").cloned().unwrap_or_default();
                            out.push_str(&super::render::render_top_events(&events, 8));
                        }
                        print!("{}", out);
                        let _ = std::io::stdout().flush();
                        let code = task["exit_code"].as_i64().unwrap_or(0);
                        std::process::exit(code as i32);
                    }
                }
            }
            Err(_) => {
                // Daemon vanished mid-task — the task may still be running.
                println!(
                    "⟳ task {} still running (daemon unreachable) — query: `arshy query {}`",
                    task_id, task_id
                );
                std::process::exit(0);
            }
        }

        if start.elapsed() >= patience {
            println!(
                "⟳ task {} still running — delegated. Poll: `arshy query {}`, kill: `arshy kill {}`",
                task_id, task_id, task_id
            );
            std::process::exit(0);
        }
        tokio::time::sleep(poll_interval).await;
    }
}

fn execute_real_shell(exe_name: &str, args: &[String]) -> Result<()> {
    let real_shell_path = match exe_name {
        "bash" => "/bin/bash",
        "zsh" => "/bin/zsh",
        _ => "/bin/sh",
    };

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(real_shell_path);
        cmd.args(&args[1..]);
        let err = cmd.exec();
        eprintln!("Failed to execute real shell {}: {}", real_shell_path, err);
        std::process::exit(1);
    }

    #[cfg(not(unix))]
    {
        let mut child = std::process::Command::new(real_shell_path).args(&args[1..]).spawn()?;
        let status = child.wait()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn get_parent_process_name() -> Option<String> {
    #[cfg(unix)]
    {
        let ppid = unsafe { libc::getppid() };
        let output = std::process::Command::new("ps")
            .args(["-p", &ppid.to_string(), "-o", "comm="])
            .output()
            .ok()?;
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Some(name)
        } else {
            None
        }
    }
    #[cfg(not(unix))]
    {
        None
    }
}

// ── Hook installers ──────────────────────────────────────────────────────────

/// Returns true if the arshy shell-hook shims are active in the current shell's PATH,
/// i.e. a plain `bash`/`zsh` invocation resolves to the arshy wrapper instead of the
/// real shell. Used by `arshy doctor` to report whether agent bash calls actually
/// reach arshy (and thus whether dogfooding is happening).
pub(crate) fn is_hook_active() -> bool {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return false,
    };
    let bin_dir = home.join(".arshy").join("bin");
    if !bin_dir.is_dir() {
        return false;
    }
    // The shim must exist and point at the arshy binary.
    let shim = bin_dir.join("bash");
    if !shim.exists() {
        return false;
    }
    let points_at_arshy = std::fs::read_link(&shim)
        .map(|t| t.file_name().and_then(|s| s.to_str()) == Some("arshy"))
        .unwrap_or(false);
    if !points_at_arshy {
        return false;
    }
    // The shim dir must precede the system shell dirs in PATH so it shadows `bash`.
    let path = std::env::var("PATH").unwrap_or_default();
    let dirs: Vec<&str> = path.split(':').collect();
    let our_pos = dirs.iter().position(|d| std::path::Path::new(d) == bin_dir);
    let sys_pos = dirs.iter().position(|d| *d == "/bin" || *d == "/usr/bin");
    match (our_pos, sys_pos) {
        (Some(o), Some(s)) => o < s,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Resolve the arshy binary that shims / MCP entries should point at.
/// Prefers an installed binary (outside `target/`) over the running debug build.
pub(crate) fn resolve_arshy_bin() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    for candidate in [
        home.join(".local").join("bin").join("arshy"),
        PathBuf::from("/usr/local/bin/arshy"),
        PathBuf::from("/opt/homebrew/bin/arshy"),
    ] {
        if candidate.exists() {
            return candidate;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if !exe.to_string_lossy().contains("target") {
            return exe;
        }
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("arshy"))
}

/// Directory that holds the shim symlinks (`~/.arshy/bin`).
pub(crate) fn arshy_bin_dir(home: &Path) -> PathBuf {
    home.join(".arshy").join("bin")
}

/// Create `~/.arshy/bin/{sh,bash,zsh,claude-hook}` symlinks pointing at the
/// resolved arshy binary (L0). Idempotent.
pub(crate) fn setup_shims(arshy_bin: &Path) -> Result<()> {
    let home = dirs::home_dir()
        .ok_or_else(|| arshy_lib::ArshyError::DaemonUnreachable("HOME not set".into()))?;
    let bin_dir = arshy_bin_dir(&home);
    std::fs::create_dir_all(&bin_dir)?;

    let shells = ["sh", "bash", "zsh", "claude-hook"];
    for shell in &shells {
        let symlink_path = bin_dir.join(shell);
        if symlink_path.exists() {
            let _ = std::fs::remove_file(&symlink_path);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(arshy_bin, &symlink_path)?;
    }
    Ok(())
}

/// Prepend `~/.arshy/bin` to `~/.zshrc` / `~/.bashrc` (L1). Idempotent.
pub(crate) fn setup_terminal_path() -> Result<()> {
    let home = std::env::var("HOME")
        .map_err(|_| arshy_lib::ArshyError::DaemonUnreachable("HOME env var not set".into()))?;
    let hook_line = "export PATH=\"$HOME/.arshy/bin:$PATH\" # Arshy shell integration hook";
    let rc_files = [".zshrc", ".bashrc"];
    for rc in &rc_files {
        let rc_path = PathBuf::from(&home).join(rc);
        if rc_path.exists() {
            let content = std::fs::read_to_string(&rc_path)?;
            if !content.contains("Arshy shell integration hook") {
                let mut file = std::fs::OpenOptions::new().append(true).open(&rc_path)?;
                writeln!(file, "\n{}", hook_line)?;
                println!("Added hook path to {}", rc_path.display());
            }
        }
    }
    Ok(())
}

pub fn install_hook() -> Result<()> {
    let arshy_bin = resolve_arshy_bin();
    setup_shims(&arshy_bin)?;
    setup_terminal_path()?;

    // Also register the Claude Code PreToolUse hook for first-class support.
    let home = std::env::var("HOME")
        .map_err(|_| arshy_lib::ArshyError::DaemonUnreachable("HOME env var not set".into()))?;
    let claude_hook_bin = arshy_bin.parent().unwrap_or(&arshy_bin);
    let _ = install_claude_hook(&home, claude_hook_bin);

    let home_path = PathBuf::from(&home);
    let bin_dir = arshy_bin_dir(&home_path);
    let parsers_dir = home_path.join(".arshy").join("parsers");
    std::fs::create_dir_all(&parsers_dir).ok();
    println!("Shell hook successfully installed!");
    println!("Prepend directory: {}", bin_dir.display());
    println!(
        "Custom parser directory created: {} (drop your custom TOML rules here)",
        parsers_dir.display()
    );
    println!("Please restart your terminal or run: source ~/.zshrc");
    Ok(())
}

pub fn uninstall_hook() -> Result<()> {
    let home = std::env::var("HOME")
        .map_err(|_| arshy_lib::ArshyError::DaemonUnreachable("HOME env var not set".into()))?;
    let arshy_bin_dir = PathBuf::from(&home).join(".arshy").join("bin");

    // Remove symlinks
    let shells = ["sh", "bash", "zsh", "claude-hook"];
    for shell in &shells {
        let symlink_path = arshy_bin_dir.join(shell);
        if symlink_path.exists() {
            let _ = std::fs::remove_file(&symlink_path);
        }
    }

    // Remove ~/.arshy/bin if empty
    if arshy_bin_dir.exists() {
        let _ = std::fs::remove_dir(&arshy_bin_dir);
    }

    // Remove from ~/.zshrc and ~/.bashrc
    let rc_files = [".zshrc", ".bashrc"];
    for rc in &rc_files {
        let rc_path = PathBuf::from(&home).join(rc);
        if rc_path.exists() {
            let content = std::fs::read_to_string(&rc_path)?;
            let mut new_lines = Vec::new();
            let mut modified = false;
            for line in content.lines() {
                if line.contains("Arshy shell integration hook") {
                    modified = true;
                } else {
                    new_lines.push(line);
                }
            }
            if modified {
                let new_content = new_lines.join("\n") + "\n";
                std::fs::write(&rc_path, new_content)?;
                println!("Removed hook path from {}", rc_path.display());
            }
        }
    }

    // Also automatically remove settings hook for Claude Code
    let _ = uninstall_claude_hook(&home);

    println!("Shell hook successfully uninstalled!");
    Ok(())
}

/// Initialize arshy in the current workspace — the **three-layer protocol
/// path** (agent-agnostic, no per-agent list needed):
///
/// 1. `.arshy.toml` — bash-proxy opt-in marker (intercept `bash -c` here,
///    auto-start the daemon on demand)
/// 2. `.mcp.json` — project-level MCP registration; **any** MCP-capable agent
///    that reads project MCP configs picks up `arshy --from-mcp` without
///    knowing which agent it is
/// 3. `AGENTS.md` — arshy instruction block (route shell commands through
///    `arshy_exec`), idempotently injected with the shared marker
///
/// All three layers are idempotent and reversible (`arshy init --undo`).
pub fn init_workspace() -> Result<()> {
    let cwd = std::env::current_dir().map_err(|e| {
        arshy_lib::ArshyError::DaemonUnreachable(format!("cannot determine cwd: {e}"))
    })?;
    let actions = init_workspace_at(&cwd)?;
    println!("Initialized arshy workspace: {}", cwd.display());
    for a in &actions {
        println!("  {a}");
    }
    println!(
        "Any MCP-capable agent in this project now sees arshy_exec/arshy_query;          the bash proxy is active for agent `bash -c` calls."
    );
    println!("Undo with: `arshy init --undo`");
    Ok(())
}

/// Testable core of `init_workspace` — returns the action summary.
pub(crate) fn init_workspace_at(cwd: &Path) -> Result<Vec<String>> {
    let mut actions: Vec<String> = Vec::new();

    // Layer 1: bash-proxy opt-in marker (arshy-owned file)
    let marker = cwd.join(".arshy.toml");
    if marker.exists() {
        actions.push(format!("✓ {} (already present)", marker.display()));
    } else {
        let content = "# Arshy workspace opt-in marker\n\
                       # Presence of this file (or an .arshy/ directory) in a workspace tells\n\
                       # arshy's shell hook to intercept commands here and start the daemon on demand.\n\
                       enabled = true\n";
        std::fs::write(&marker, content).map_err(|e| {
            arshy_lib::ArshyError::DaemonUnreachable(format!(
                "failed to write {}: {}",
                marker.display(),
                e
            ))
        })?;
        actions.push(format!("✓ {}", marker.display()));
    }

    // Layer 2: project-level MCP registration — agent-agnostic by design.
    // `command` stays "arshy" (PATH-resolved) so the file is portable when
    // committed to the repo; note in the summary if PATH lookup matters.
    let mcp_path = cwd.join(".mcp.json");
    let mcp_actions =
        crate::cli::integrate::merge_mcp_json(&mcp_path, "arshy", Path::new("arshy"), false)?;
    actions.extend(mcp_actions);

    // Layer 3: AGENTS.md instruction block (shared marker, idempotent)
    let agents_md = cwd.join("AGENTS.md");
    let md_actions = crate::cli::integrate::inject_agents_md(&agents_md, false)?;
    actions.extend(md_actions);

    Ok(actions)
}

/// Undo `init_workspace`: remove arshy's three project-layer artifacts with
/// zero residue (`.arshy.toml` is arshy-owned and deleted; `.mcp.json` and
/// `AGENTS.md` keep user content, only the arshy entry/section is removed).
pub fn init_workspace_undo() -> Result<()> {
    let cwd = std::env::current_dir().map_err(|e| {
        arshy_lib::ArshyError::DaemonUnreachable(format!("cannot determine cwd: {e}"))
    })?;
    let actions = init_workspace_undo_at(&cwd)?;
    println!("Removed arshy from workspace: {}", cwd.display());
    for a in &actions {
        println!("  {a}");
    }
    Ok(())
}

/// Testable core of `init_workspace_undo`.
pub(crate) fn init_workspace_undo_at(cwd: &Path) -> Result<Vec<String>> {
    let mut actions: Vec<String> = Vec::new();

    let marker = cwd.join(".arshy.toml");
    if marker.exists() {
        std::fs::remove_file(&marker).map_err(|e| {
            arshy_lib::ArshyError::DaemonUnreachable(format!(
                "failed to remove {}: {}",
                marker.display(),
                e
            ))
        })?;
        actions.push(format!("✓ removed {}", marker.display()));
    } else {
        actions.push(format!("· {} not present", marker.display()));
    }

    actions.extend(crate::cli::integrate::remove_mcp_json(&cwd.join(".mcp.json"), "arshy", false)?);
    actions.extend(crate::cli::integrate::remove_agents_md(&cwd.join("AGENTS.md"), false)?);

    Ok(actions)
}

pub(crate) fn install_claude_hook(home: &str, arshy_bin_dir: &std::path::Path) -> Result<()> {
    let settings_path = std::path::PathBuf::from(home).join(".claude").join("settings.json");
    if !settings_path.exists() {
        return Ok(());
    }

    let file_content = std::fs::read_to_string(&settings_path)?;
    let mut settings: serde_json::Value = match serde_json::from_str(&file_content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to parse ~/.claude/settings.json: {}", e);
            return Ok(());
        }
    };

    let settings_obj = match settings.as_object_mut() {
        Some(o) => o,
        None => return Ok(()),
    };

    let claude_hook_path = arshy_bin_dir.join("claude-hook").to_string_lossy().to_string();
    let new_hook = serde_json::json!({
        "type": "command",
        "command": claude_hook_path
    });

    let hooks = settings_obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks_obj = match hooks.as_object_mut() {
        Some(o) => o,
        None => return Ok(()),
    };

    let pre_tool_use = hooks_obj.entry("PreToolUse").or_insert_with(|| serde_json::json!([]));

    if let Some(arr) = pre_tool_use.as_array_mut() {
        let mut bash_entry_idx = None;
        for (idx, entry) in arr.iter().enumerate() {
            if entry.get("matcher").and_then(|v| v.as_str()) == Some("Bash") {
                bash_entry_idx = Some(idx);
                break;
            }
        }

        match bash_entry_idx {
            Some(idx) => {
                let entry = &mut arr[idx];
                let entry_obj = match entry.as_object_mut() {
                    Some(o) => o,
                    None => return Ok(()),
                };
                let inner_hooks = entry_obj.entry("hooks").or_insert_with(|| serde_json::json!([]));
                if let Some(inner_arr) = inner_hooks.as_array_mut() {
                    let already_installed = inner_arr.iter().any(|h| {
                        h.get("command")
                            .and_then(|c| c.as_str())
                            .is_some_and(|c| c.contains("claude-hook"))
                    });
                    if !already_installed {
                        inner_arr.push(new_hook);
                        println!("Registered Claude Code PreToolUse hook.");
                    }
                }
            }
            None => {
                arr.push(serde_json::json!({
                    "matcher": "Bash",
                    "hooks": [new_hook]
                }));
                println!("Registered Claude Code PreToolUse hook.");
            }
        }
    }

    let updated_content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, updated_content)?;
    Ok(())
}

pub(crate) fn uninstall_claude_hook(home: &str) -> Result<()> {
    let settings_path = std::path::PathBuf::from(home).join(".claude").join("settings.json");
    if !settings_path.exists() {
        return Ok(());
    }

    let file_content = std::fs::read_to_string(&settings_path)?;
    let mut settings: serde_json::Value = match serde_json::from_str(&file_content) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    let mut modified = false;

    if let Some(hooks) = settings.get_mut("hooks") {
        if let Some(pre_tool_use) = hooks.get_mut("PreToolUse") {
            if let Some(arr) = pre_tool_use.as_array_mut() {
                let mut to_remove = Vec::new();
                for (idx, entry) in arr.iter_mut().enumerate() {
                    if entry.get("matcher").and_then(|v| v.as_str()) == Some("Bash") {
                        if let Some(inner_hooks) = entry.get_mut("hooks") {
                            if let Some(inner_arr) = inner_hooks.as_array_mut() {
                                let len_before = inner_arr.len();
                                inner_arr.retain(|h| {
                                    !h.get("command")
                                        .and_then(|c| c.as_str())
                                        .is_some_and(|c| c.contains("claude-hook"))
                                });
                                if inner_arr.len() != len_before {
                                    modified = true;
                                }
                            }
                        }
                        // If inner hooks is empty, mark this entry for removal
                        if entry
                            .get("hooks")
                            .and_then(|h| h.as_array())
                            .is_none_or(|a| a.is_empty())
                        {
                            to_remove.push(idx);
                        }
                    }
                }

                // Remove empty entries in reverse order
                for idx in to_remove.into_iter().rev() {
                    arr.remove(idx);
                    modified = true;
                }
            }

            // If PreToolUse is empty, remove it
            if pre_tool_use.as_array().is_none_or(|a| a.is_empty()) {
                if let Some(obj) = hooks.as_object_mut() {
                    obj.remove("PreToolUse");
                    modified = true;
                }
            }
        }

        // If hooks is empty, remove it
        if hooks.as_object().is_none_or(|o| o.is_empty()) {
            if let Some(obj) = settings.as_object_mut() {
                obj.remove("hooks");
                modified = true;
            }
        }
    }

    if modified {
        let updated_content = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&settings_path, updated_content)?;
        println!("Deregistered Claude Code PreToolUse hook.");
    }
    Ok(())
}

/// Handler for Claude Code's PreToolUse hook.
/// Reads the input JSON from stdin, and if it's a Bash command not already calling arshy,
/// rewrites it to use arshy run with ARSHY_BYPASS set to prevent recursive interception.
/// Always outputs valid JSON to stdout.
pub fn run_claude_hook() -> Result<()> {
    use std::io::{self, Read};
    let mut buffer = String::new();
    let mut stdin = io::stdin();
    if stdin.read_to_string(&mut buffer).is_err() {
        print_claude_allow(None);
        return Ok(());
    }

    let input_val: serde_json::Value = match serde_json::from_str(&buffer) {
        Ok(v) => v,
        Err(_) => {
            print_claude_allow(None);
            return Ok(());
        }
    };

    let tool_name = input_val.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    let command =
        input_val.get("tool_input").and_then(|t| t.get("command")).and_then(|v| v.as_str());

    if tool_name == "Bash" {
        if let Some(cmd) = command {
            let cmd_trim = cmd.trim();
            // Prevent recursive interception
            let is_arshy = cmd_trim.starts_with("arshy")
                || cmd_trim.starts_with("arshyd")
                || cmd_trim.contains("/arshy")
                || cmd_trim.contains("claude-hook");

            if !is_arshy {
                let escaped = escape_shell_arg(cmd);
                // Set ARSHY_BYPASS=1 in the environment to prevent re-interception
                // when this command runs through shell_wrapper again
                let rewritten = format!("ARSHY_BYPASS=1 arshy run {}", escaped);
                print_claude_allow(Some(&rewritten));
                return Ok(());
            }
        }
    }

    print_claude_allow(None);
    Ok(())
}

fn print_claude_allow(rewritten_command: Option<&str>) {
    let response = if let Some(cmd) = rewritten_command {
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {
                    "command": cmd
                }
            }
        })
    } else {
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow"
            }
        })
    };
    if let Ok(json_str) = serde_json::to_string(&response) {
        println!("{}", json_str);
    }
}

/// Escapes a shell argument using POSIX single quotes.
fn escape_shell_arg(arg: &str) -> String {
    let mut escaped = String::new();
    escaped.push('\'');
    for c in arg.chars() {
        if c == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(c);
        }
    }
    escaped.push('\'');
    escaped
}

// ── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Convenience: extract the boolean from an InterceptDecision.
    fn intercepts(
        args: &[String],
        env: impl Fn(&str) -> bool,
        parent: &Option<String>,
        ws: bool,
    ) -> bool {
        should_intercept(args, env, parent, ws).intercept
    }

    #[test]
    fn test_init_workspace_three_layers_idempotent_and_undo() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let cwd = dir.path();

        // First init: all three layers created.
        init_workspace_at(cwd).unwrap();
        assert!(cwd.join(".arshy.toml").is_file(), "bash-proxy marker");
        assert!(cwd.join(".mcp.json").is_file(), "project MCP registration");
        assert!(cwd.join("AGENTS.md").is_file(), "instruction block");
        let mcp = std::fs::read_to_string(cwd.join(".mcp.json")).unwrap();
        assert!(mcp.contains("\"arshy\""), "mcp.json registers the arshy server");
        assert!(mcp.contains("--from-mcp"), "mcp.json points at the stdio proxy");
        let agents = std::fs::read_to_string(cwd.join("AGENTS.md")).unwrap();
        assert!(agents.contains("arshy-agent-instructions"), "AGENTS.md carries the marker");

        // Second init: idempotent — no duplicate sections/entries.
        let actions2 = init_workspace_at(cwd).unwrap();
        assert!(actions2.join("\n").contains("already present"), "marker reused");
        let agents2 = std::fs::read_to_string(cwd.join("AGENTS.md")).unwrap();
        assert_eq!(
            agents2.matches("arshy-agent-instructions").count(),
            2,
            "marker appears exactly once as an open/close pair"
        );
        let mcp2 = std::fs::read_to_string(cwd.join(".mcp.json")).unwrap();
        assert_eq!(
            mcp2.matches("\"arshy\": {").count(),
            1,
            "no duplicate server entry (key appears once; the command value is expected twice)"
        );

        // Undo: arshy artifacts gone, user content preserved.
        let undo = init_workspace_undo_at(cwd).unwrap();
        assert!(!cwd.join(".arshy.toml").exists(), ".arshy.toml removed");
        // The .mcp.json may be removed entirely when its only content was the
        // arshy entry (zero-residue empty-shell deletion) — either way it must
        // no longer mention arshy.
        let mcp_path = cwd.join(".mcp.json");
        if mcp_path.exists() {
            let mcp3 = std::fs::read_to_string(&mcp_path).unwrap();
            assert!(!mcp3.contains("arshy"), "mcp.json entry removed");
        }
        let agents_path = cwd.join("AGENTS.md");
        if agents_path.exists() {
            let agents3 = std::fs::read_to_string(&agents_path).unwrap();
            assert!(!agents3.contains("arshy-agent-instructions"), "instruction block removed");
        }
        assert!(undo.len() >= 3, "undo reports each layer");
    }

    #[test]
    fn test_should_intercept_interactive_no_c() {
        let args = vec!["sh".to_string(), "script.sh".to_string()];
        let env_check = |_key: &str| false;
        assert!(!intercepts(&args, env_check, &None, true));
    }

    #[test]
    fn test_should_intercept_non_interactive_no_agent() {
        let args = vec!["sh".to_string(), "-c".to_string(), "echo hello".to_string()];
        let env_check = |_key: &str| false;
        assert!(!intercepts(&args, env_check, &None, true));
        assert!(!intercepts(&args, env_check, &Some("bash".to_string()), true));
    }

    #[test]
    fn test_should_intercept_agent_env() {
        let args = vec!["sh".to_string(), "-c".to_string(), "cargo build".to_string()];

        let env_check_arshy = |key: &str| key == "ARSHY_AGENT";
        assert!(intercepts(&args, env_check_arshy, &None, true));

        let env_check_claude = |key: &str| key == "CLAUDE_CODE";
        assert!(intercepts(&args, env_check_claude, &None, true));
    }

    #[test]
    fn test_should_intercept_agent_parent() {
        let args = vec!["sh".to_string(), "-c".to_string(), "npm test".to_string()];
        let env_check = |_key: &str| false;

        assert!(intercepts(&args, env_check, &Some("Cursor".to_string()), true));
        assert!(intercepts(
            &args,
            env_check,
            &Some("/Applications/VSCode.app/Contents/MacOS/Electron".to_string()),
            true
        ));
        assert!(intercepts(&args, env_check, &Some("claude-code".to_string()), true));
        assert!(intercepts(&args, env_check, &Some("antigravity".to_string()), true));
        assert!(intercepts(&args, env_check, &Some("gemini-agent".to_string()), true));
        assert!(intercepts(&args, env_check, &Some("agy".to_string()), true));
    }

    #[test]
    fn test_should_intercept_agent_but_workspace_not_configured() {
        // Ambiguous agent signals (env marker, generic "node"/"agent" parents)
        // must NOT intercept without a workspace opt-in — we never want to
        // surprise ordinary scripts or auto-start the daemon by accident.
        let args = vec!["sh".to_string(), "-c".to_string(), "cargo build".to_string()];
        let env_check_arshy = |key: &str| key == "ARSHY_AGENT";
        assert!(!intercepts(&args, env_check_arshy, &None, false));
        assert!(!intercepts(&args, |_k: &str| false, &Some("node".to_string()), false));
        assert!(!intercepts(&args, |_k: &str| false, &Some("python".to_string()), false));
    }

    #[test]
    fn test_should_intercept_known_agent_without_workspace() {
        // Unambiguous desktop agents get intercepted even without a workspace
        // marker — this is what makes the bash proxy take over agent shells
        // with zero per-project setup.
        let args = vec!["sh".to_string(), "-c".to_string(), "cargo build".to_string()];
        let env_check = |_key: &str| false;
        for parent in [
            "codex",
            "claude-code",
            "Cursor",
            "Code Helper",
            "gemini-agent",
            "antigravity",
            "opencode",
            "aider",
            "workbuddy",
        ] {
            assert!(
                intercepts(&args, env_check, &Some(parent.to_string()), false),
                "parent {} should intercept without workspace opt-in",
                parent
            );
        }
    }

    #[test]
    fn test_should_intercept_reports_reasons() {
        let args = vec!["sh".to_string(), "-c".to_string(), "npm test".to_string()];
        // No agent signal at all, no workspace → pass with an explicit reason.
        let d = should_intercept(&args, |_k: &str| false, &None, false);
        assert!(!d.intercept);
        assert_eq!(d.reason, "no workspace opt-in and not a known agent parent");
        // Env marker → intercept.
        let d = should_intercept(&args, |k: &str| k == "ARSHY_AGENT", &None, true);
        assert!(d.intercept);
        assert_eq!(d.reason, "agent env marker");
        // Known agent parent without workspace → intercept.
        let d = should_intercept(&args, |_k: &str| false, &Some("codex".to_string()), false);
        assert!(d.intercept);
        assert_eq!(d.reason, "known agent parent");
        // Ambiguous parent requires the workspace opt-in.
        let d = should_intercept(&args, |_k: &str| false, &Some("node".to_string()), true);
        assert!(d.intercept);
        assert_eq!(d.reason, "ambiguous agent parent with workspace opt-in");
        let d = should_intercept(&args, |_k: &str| false, &Some("node".to_string()), false);
        assert!(!d.intercept);
        // TTY conflict beats everything.
        let d = should_intercept(&args, |_k: &str| false, &Some("vim".to_string()), true);
        assert!(!d.intercept);
        assert_eq!(d.reason, "tty conflict (vim/less/tmux...)");
        // Recursive call passes through.
        let recursive = vec!["sh".to_string(), "-c".to_string(), "arshy run x".to_string()];
        let d = should_intercept(&recursive, |_k: &str| false, &None, true);
        assert!(!d.intercept);
        assert_eq!(d.reason, "recursive call (arshy/claude-hook in args)");
    }

    #[test]
    fn test_is_workspace_configured_marker_in_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_workspace_configured(tmp.path()));
        std::fs::write(tmp.path().join(".arshy.toml"), "enabled = true\n").unwrap();
        assert!(is_workspace_configured(tmp.path()));
    }

    #[test]
    fn test_is_workspace_configured_marker_in_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(tmp.path().join(".arshy.toml"), "enabled = true\n").unwrap();
        assert!(is_workspace_configured(&nested));
    }

    #[test]
    fn test_is_workspace_configured_via_arshy_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_workspace_configured(tmp.path()));
        std::fs::create_dir_all(tmp.path().join(".arshy")).unwrap();
        assert!(is_workspace_configured(tmp.path()));
    }

    #[test]
    fn test_is_workspace_configured_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_workspace_configured(tmp.path()));
    }

    #[test]
    fn test_escape_shell_arg_formatting() {
        assert_eq!(escape_shell_arg("cargo test"), "'cargo test'");
        assert_eq!(escape_shell_arg("echo \"hello\""), "'echo \"hello\"'");
        assert_eq!(escape_shell_arg("echo 'hello'"), "'echo '\\''hello'\\'''");
        assert_eq!(escape_shell_arg("echo $VAR"), "'echo $VAR'");
    }

    #[test]
    fn test_claude_hook_install_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        let settings_path = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();

        // Write initial empty/simple settings
        let initial_json = serde_json::json!({
            "effortLevel": "medium"
        });
        std::fs::write(&settings_path, serde_json::to_string_pretty(&initial_json).unwrap())
            .unwrap();

        // Install hook
        let home_str = tmp.path().to_string_lossy().to_string();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        install_claude_hook(&home_str, &bin_dir).unwrap();

        // Verify it was written
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let hooks = parsed.get("hooks").unwrap();
        let pre_tool_use = hooks.get("PreToolUse").unwrap().as_array().unwrap();
        assert_eq!(pre_tool_use.len(), 1);
        let first = &pre_tool_use[0];
        assert_eq!(first.get("matcher").unwrap().as_str().unwrap(), "Bash");
        let inner_hooks = first.get("hooks").unwrap().as_array().unwrap();
        assert_eq!(inner_hooks.len(), 1);
        assert!(inner_hooks[0].get("command").unwrap().as_str().unwrap().contains("claude-hook"));

        // Uninstall hook
        uninstall_claude_hook(&home_str).unwrap();

        // Verify settings back to clean
        let content_after = std::fs::read_to_string(&settings_path).unwrap();
        let parsed_after: serde_json::Value = serde_json::from_str(&content_after).unwrap();
        assert!(parsed_after.get("hooks").is_none());
    }
}
