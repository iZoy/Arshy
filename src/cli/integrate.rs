//! Multi-agent integration — the elegant model: **two paths, one command**.
//!
//! 1. **Native path (MCP / hook)** — every known agent gets arshy registered
//!    as an MCP server (plus instructions / hooks where supported). The agent
//!    calls `arshy_exec` directly and receives structured results with zero
//!    runtime overhead. This is the preferred, "no-friction" path.
//! 2. **Fallback path (transparent bash proxy)** — `~/.arshy/bin/bash` shims
//!    intercept `bash -c` spawned by agents at the process level and route them
//!    through the same execution layer. Humans and unknown processes pass
//!    through untouched. See `shell_wrapper.rs`.
//!
//! What we deliberately do NOT do:
//! - No per-agent shim/PATH engineering (the proxy is one mechanism, activated
//!   by agent-process detection, not by editing each agent's environment).
//! - No Codex PreToolUse deny-redirect: Codex's PreToolUse only supports
//!   deny/block (no `updatedInput` rewrite), so a redirect would interrupt the
//!   agent instead of being seamless. Codex is covered by MCP + instructions
//!   (native) and the bash proxy (fallback).
//!
//! Design constraints (project red lines):
//! - Never touch system binaries (`/bin/bash`, etc.). Changes are limited to
//!   `~/.arshy/`, each agent's dotfiles, `~/Library/LaunchAgents`, and
//!   `~/.config/environment.d`.
//! - Idempotent, reversible (every `integrate` has a matching `uninstall`),
//!   and a `--dry-run` mode that reports intent without side effects.
//!
//! See `docs/explanation/integration-model.md` for the full rationale.

use arshy_lib::Result;
use std::path::{Path, PathBuf};

/// Marker that bounds the arshy section injected into AGENTS.md / CLAUDE.md style files,
/// so `uninstall` can remove it precisely and `integrate` can detect it idempotently.
const ARSHY_AGENTS_MD_MARKER: &str = "<!-- arshy-agent-instructions -->";

/// Per-agent diagnostic record consumed by `doctor` (L4).
pub struct AgentStatus {
    pub id: &'static str,
    pub name: String,
    pub mechanism: String,
    pub detected: bool,
    pub active: bool,
    pub detail: String,
}

/// A known agent environment and how arshy wires into it.
pub trait AgentIntegration {
    /// Stable identifier (e.g. `claude-code`), used by `arshy integrate --agent <id>`.
    fn id(&self) -> &'static str;
    /// Human-readable name.
    fn name(&self) -> &'static str;
    /// Mechanism label shown in doctor (`MCP`, `PreToolUse`, `AGENTS.md`, `GUI PATH`).
    fn mechanism(&self) -> &'static str;
    /// Is this agent installed on this machine?
    fn detect(&self, home: &Path) -> bool;
    /// Apply arshy integration. Returns human-readable actions taken.
    fn integrate(&self, home: &Path, arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>>;
    /// Remove arshy integration (best-effort, idempotent).
    fn uninstall(&self, home: &Path, dry_run: bool) -> Result<Vec<String>>;
    /// Diagnostic snapshot for doctor.
    fn status(&self, home: &Path, arshy_bin: &Path, cwd: &Path) -> AgentStatus;
}

// ── Registry ────────────────────────────────────────────────────────────────

/// All supported agents. Order is the priority order for `integrate --all`.
pub fn all_agents() -> Vec<Box<dyn AgentIntegration>> {
    vec![
        Box::new(ClaudeCode),
        Box::new(Cursor),
        Box::new(VsCode),
        Box::new(Antigravity),
        Box::new(Codex),
        Box::new(OpenCode),
        Box::new(Aider),
        Box::new(WorkBuddy),
    ]
}

// ── L2: GUI session PATH persistence ─────────────────────────────────────────

/// Add `~/.arshy/bin` to the GUI session PATH so desktop-launched agents
/// (WorkBuddy, Cursor.app, VS Code.app) inherit it even though they never
/// source `.zshrc`/`.bashrc`.
///
/// - macOS: `launchctl setenv PATH` (immediate) + a `LaunchAgent` that re-applies
///   it at login (persistent). `launchctl setenv` alone does NOT survive reboot.
/// - Linux: `~/.config/environment.d/arshy.conf` (picked up by systemd user sessions).
pub fn setup_gui_path(dry_run: bool) -> Result<Vec<String>> {
    let home =
        dirs::home_dir().ok_or_else(|| arshy_lib::ArshyError::Other("HOME is not set".into()))?;
    let arshy_bin_dir = crate::cli::shell_wrapper::arshy_bin_dir(&home);
    let mut actions = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library").join("LaunchAgents").join("com.arshy.path.plist");
        let xml = gui_plist_xml(&arshy_bin_dir);
        if !dry_run {
            if let Some(parent) = plist.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&plist, &xml)?;
            let new_path = prepend_path(&current_launchd_path(), &arshy_bin_dir.to_string_lossy());
            if std::process::Command::new("launchctl")
                .args(["setenv", "PATH", &new_path])
                .output()
                .is_err()
            {
                actions.push("⚠ launchctl setenv failed (PATH not updated this session)".into());
            }
            let _ =
                std::process::Command::new("launchctl").args(["load", "-w"]).arg(&plist).output();
        }
        actions.push(format!(
            "macOS: prepend {} to launchd PATH + LaunchAgent {}",
            arshy_bin_dir.display(),
            plist.display()
        ));
    }

    #[cfg(target_os = "linux")]
    {
        let envd_dir = home.join(".config").join("environment.d");
        let envd = envd_dir.join("arshy.conf");
        let content = gui_envd_content(&arshy_bin_dir);
        if !dry_run {
            std::fs::create_dir_all(&envd_dir)?;
            std::fs::write(&envd, content)?;
        }
        actions.push(format!("Linux: wrote {}", envd.display()));
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        actions.push(
            "GUI PATH persistence is not supported on this OS; relying on terminal PATH (L1)."
                .into(),
        );
    }

    Ok(actions)
}

/// Reverse `setup_gui_path`: unload the LaunchAgent / remove env.d and restore
/// the prior PATH.
pub fn teardown_gui_path(dry_run: bool) -> Result<Vec<String>> {
    let home =
        dirs::home_dir().ok_or_else(|| arshy_lib::ArshyError::Other("HOME is not set".into()))?;
    let arshy_bin_dir = crate::cli::shell_wrapper::arshy_bin_dir(&home);
    let mut actions = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library").join("LaunchAgents").join("com.arshy.path.plist");
        if !dry_run {
            if plist.exists() {
                let _ =
                    std::process::Command::new("launchctl").args(["unload"]).arg(&plist).output();
                let _ = std::fs::remove_file(&plist);
            }
            // Restore the prior PATH (stripping our dir) so we never leave a stale value.
            let cur = current_launchd_path();
            let restored = prepend_path(&cur, &arshy_bin_dir.to_string_lossy());
            let _ = std::process::Command::new("launchctl")
                .args(["setenv", "PATH", &restored])
                .output();
        }
        actions.push("macOS: removed LaunchAgent + restored launchd PATH".into());
    }

    #[cfg(target_os = "linux")]
    {
        let envd = home.join(".config").join("environment.d").join("arshy.conf");
        if !dry_run && envd.exists() {
            let _ = std::fs::remove_file(&envd);
        }
        actions.push("Linux: removed environment.d/arshy.conf".into());
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        actions.push("nothing to tear down on this OS".into());
    }

    Ok(actions)
}

/// macOS-only: read the current launchd PATH (falls back to a sane default).
#[cfg(target_os = "macos")]
fn current_launchd_path() -> String {
    if let Ok(out) = std::process::Command::new("launchctl").args(["getenv", "PATH"]).output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into()
}

/// Prepend `prepend` to a PATH string, removing any existing occurrence of it
/// (so re-running never duplicates). Returns the new PATH.
pub(crate) fn prepend_path(current: &str, prepend: &str) -> String {
    let prepend = prepend.trim_end_matches('/');
    let mut out = String::new();
    for part in current.split(':') {
        if part.is_empty() || part == prepend {
            continue; // drop empty and any existing copy of prepend
        }
        if !out.is_empty() {
            out.push(':');
        }
        out.push_str(part);
    }
    if out.is_empty() {
        prepend.to_string()
    } else {
        format!("{}:{}", prepend, out)
    }
}

/// Pure helper: plist XML for the login-time PATH setter (macOS).
#[cfg(target_os = "macos")]
pub(crate) fn gui_plist_xml(arshy_bin_dir: &Path) -> String {
    let dir = arshy_bin_dir.display();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.arshy.path</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/sh</string>
        <string>-c</string>
        <string>launchctl setenv PATH {d}:$(launchctl getenv PATH)</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        d = dir
    )
}

/// Pure helper: environment.d content (Linux systemd user sessions).
#[cfg(target_os = "linux")]
pub(crate) fn gui_envd_content(arshy_bin_dir: &Path) -> String {
    format!("{}:$PATH\n", arshy_bin_dir.display())
}

// ── Shared integration helpers ──────────────────────────────────────────────

/// Merge an arshy MCP server entry into a `mcpServers` JSON file (idempotent).
pub(crate) fn merge_mcp_json(
    path: &Path,
    server_name: &str,
    arshy_bin: &Path,
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut data: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let arshy_bin_str = arshy_bin.to_string_lossy().to_string();
    let entry = serde_json::json!({
        "type": "stdio",
        "command": arshy_bin_str,
        "args": ["--from-mcp"],
        "env": {}
    });

    let servers = data
        .as_object_mut()
        .map(|o| o.entry("mcpServers").or_insert_with(|| serde_json::json!({})))
        .and_then(|v| v.as_object_mut());

    let changed = match servers {
        Some(map) => {
            let existing = map
                .get(server_name)
                .and_then(|v| v.get("command"))
                .map(|c| c.as_str() == Some(arshy_bin_str.as_str()))
                .unwrap_or(false);
            if existing {
                false
            } else {
                map.insert(server_name.to_string(), entry);
                true
            }
        }
        None => false,
    };

    if changed && !dry_run {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
    }

    let msg = if changed {
        format!("MCP server `{}` → {}", server_name, path.display())
    } else {
        format!("MCP server `{}` already configured in {}", server_name, path.display())
    };
    Ok(vec![msg])
}

/// Remove an arshy MCP server entry from a JSON file (idempotent).
pub(crate) fn remove_mcp_json(
    path: &Path,
    server_name: &str,
    dry_run: bool,
) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path)?;
    let mut data: serde_json::Value =
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
    let mut removed = false;
    if let Some(obj) = data.as_object_mut() {
        if let Some(servers) = obj.get_mut("mcpServers") {
            if let Some(map) = servers.as_object_mut() {
                if map.remove(server_name).is_some() {
                    removed = true;
                }
            }
        }
    }
    if removed && !dry_run {
        // Zero-residue: prune empty containers, and if nothing user-owned
        // remains, delete the file entirely (integrate created it if absent).
        if let Some(obj) = data.as_object_mut() {
            if obj.get("mcpServers").and_then(|s| s.as_object()).is_some_and(|m| m.is_empty()) {
                obj.remove("mcpServers");
            }
        }
        if data.as_object().is_some_and(|o| o.is_empty()) {
            let _ = std::fs::remove_file(path);
        } else {
            std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
        }
    }
    Ok(if removed {
        vec![format!("removed `{}` from {}", server_name, path.display())]
    } else {
        vec![]
    })
}

/// Check whether a `mcpServers` JSON file already contains the arshy entry.
fn has_mcp_json(path: &Path) -> bool {
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
    data.get("mcpServers").and_then(|s| s.get("arshy")).is_some()
}

/// Append the arshy instruction section to an AGENTS.md-style file (idempotent).
pub(crate) fn inject_agents_md(path: &Path, dry_run: bool) -> Result<Vec<String>> {
    let section = format!(
        "\n{marker}\n## Shell Execution (use arshy)\n\narshy is your shell — route shell commands through it instead of raw Bash.\n- MCP registered: use `arshy_exec(action:\"run\", command:\"<cmd>\")`; query events with `arshy_query`.\n- CLI fallback: `arshy run \"<cmd>\"` (add `--errors-only` for errors only); long tasks: `arshy query <task_id>` / `arshy kill <task_id>`.\n{marker}\n",
        marker = ARSHY_AGENTS_MD_MARKER
    );

    let existing = if path.exists() { std::fs::read_to_string(path)? } else { String::new() };

    if existing.contains(ARSHY_AGENTS_MD_MARKER) {
        return Ok(vec![format!("arshy instructions already present in {}", path.display())]);
    }

    let new_content = format!("{}{}", existing, section);
    if !dry_run {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, new_content)?;
    }
    Ok(vec![format!("injected arshy instructions into {}", path.display())])
}

/// Remove the arshy instruction section from an AGENTS.md-style file (idempotent).
pub(crate) fn remove_agents_md(path: &Path, dry_run: bool) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path)?;
    if !content.contains(ARSHY_AGENTS_MD_MARKER) {
        return Ok(vec![]);
    }
    // Strip from the first marker through the end of the closing marker (inclusive).
    let first = content.find(ARSHY_AGENTS_MD_MARKER).unwrap_or(0);
    let after_first = first + ARSHY_AGENTS_MD_MARKER.len();
    let second = content[after_first..]
        .find(ARSHY_AGENTS_MD_MARKER)
        .map(|i| after_first + i)
        .unwrap_or(content.len());
    let end = second + ARSHY_AGENTS_MD_MARKER.len();
    let new_content = format!("{}{}", &content[..first], &content[end..]);
    if !dry_run {
        if new_content.trim().is_empty() {
            // Zero-residue: the file only held our instruction block
            let _ = std::fs::remove_file(path);
        } else {
            std::fs::write(path, new_content)?;
        }
    }
    Ok(vec![format!("removed arshy instructions from {}", path.display())])
}

/// Whether a file contains the arshy instruction marker.
fn has_agents_md(path: &Path) -> bool {
    path.exists()
        && std::fs::read_to_string(path)
            .map(|c| c.contains(ARSHY_AGENTS_MD_MARKER))
            .unwrap_or(false)
}

/// Is `name` resolvable on PATH? Used by `detect()` without mutating environment.
fn which(name: &str) -> bool {
    std::env::var("PATH")
        .map(|p| p.split(':').any(|d| Path::new(d).join(name).exists()))
        .unwrap_or(false)
}

// ── Codex config.toml (TOML-based MCP registration) ──────────────────────────
//
// Codex reads MCP servers from `~/.codex/config.toml` under `[mcp_servers.*]`
// (shared by the CLI and the IDE extension). We edit this file append-only /
// line-scoped so user comments and unrelated settings are never rewritten.

/// User-level Codex config file (`~/.codex/config.toml`).
fn codex_config_path(home: &Path) -> PathBuf {
    home.join(".codex").join("config.toml")
}

/// Index of the `[mcp_servers.arshy]` table header line, if present.
fn codex_block_header_index(content: &str) -> Option<usize> {
    content.lines().position(|line| {
        let trimmed = line.trim();
        trimmed.starts_with('[')
            && trimmed.ends_with(']')
            && trimmed.chars().filter(|c| !c.is_whitespace()).collect::<String>()
                == "[mcp_servers.arshy]"
    })
}

/// Whether the Codex config already registers the arshy MCP server.
fn has_codex_mcp(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    // Prefer a real TOML parse; fall back to a tolerant line scan.
    if let Ok(value) = content.parse::<toml::Value>() {
        return value.get("mcp_servers").and_then(|s| s.get("arshy")).is_some();
    }
    codex_block_header_index(&content).is_some()
}

/// Register arshy as a Codex MCP server. Append-only: existing content,
/// comments, and other `[mcp_servers.*]` entries are preserved verbatim.
fn merge_codex_config(path: &Path, arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
    if has_codex_mcp(path) {
        return Ok(vec![format!("Codex MCP server already configured in {}", path.display())]);
    }

    let existing = if path.exists() { std::fs::read_to_string(path)? } else { String::new() };

    let block = format!(
        "\n# arshy: structured execution layer (managed by `arshy integrate`)\n[mcp_servers.arshy]\ncommand = \"{}\"\nargs = [\"--from-mcp\"]\n",
        arshy_bin.to_string_lossy()
    );

    let new_content = if existing.trim().is_empty() {
        block.trim_start_matches('\n').to_string()
    } else {
        format!("{}{}", existing, block)
    };

    if !dry_run {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, new_content)?;
    }
    Ok(vec![format!("Codex MCP server → {}", path.display())])
}

/// Remove the arshy MCP server block from the Codex config (idempotent).
fn remove_codex_config(path: &Path, dry_run: bool) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path)?;
    let header = match codex_block_header_index(&content) {
        Some(h) => h,
        None => return Ok(vec![]),
    };

    // The block ends at the next `[table]` header line, or at EOF.
    let lines: Vec<&str> = content.lines().collect();
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(header + 1) {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            end = i;
            break;
        }
    }

    // Rebuild without the block; also drop our managed `# arshy:` comment line.
    let mut kept: Vec<&str> = lines.iter().take(header).copied().collect();
    if kept.last().is_some_and(|l| l.trim_start().starts_with("# arshy:")) {
        kept.pop();
    }
    kept.extend(lines.iter().skip(end).copied());

    let mut text = kept.join("\n");
    while text.ends_with("\n\n") {
        text.pop();
    }
    if !text.is_empty() {
        text.push('\n');
    }

    if !dry_run {
        if text.trim().is_empty() {
            // Zero-residue: the config only contained our block
            let _ = std::fs::remove_file(path);
        } else {
            std::fs::write(path, text)?;
        }
    }
    Ok(vec![format!("removed Codex MCP server from {}", path.display())])
}

// ── Agent implementations ───────────────────────────────────────────────────

struct ClaudeCode;
impl AgentIntegration for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }
    fn name(&self) -> &'static str {
        "Claude Code"
    }
    fn mechanism(&self) -> &'static str {
        "PreToolUse + MCP"
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".claude.json").exists() || home.join(".claude").exists() || which("claude")
    }
    fn integrate(&self, home: &Path, arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
        let home_str = home.to_string_lossy().to_string();
        let bin_dir = arshy_bin.parent().unwrap_or(arshy_bin);
        crate::cli::shell_wrapper::install_claude_hook(&home_str, bin_dir)?;
        let mut actions = vec![format!(
            "Claude Code PreToolUse hook → {}",
            home.join(".claude").join("settings.json").display()
        )];
        // Also inject into the current workspace CLAUDE.md if present.
        if let Ok(cwd) = std::env::current_dir() {
            let claude_md = cwd.join("CLAUDE.md");
            if !dry_run || claude_md.exists() {
                actions.extend(inject_agents_md(&claude_md, dry_run)?);
            }
        }
        Ok(actions)
    }
    fn uninstall(&self, home: &Path, _dry_run: bool) -> Result<Vec<String>> {
        let home_str = home.to_string_lossy().to_string();
        crate::cli::shell_wrapper::uninstall_claude_hook(&home_str)?;
        Ok(vec!["removed Claude Code PreToolUse hook".into()])
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, _cwd: &Path) -> AgentStatus {
        let mcp = has_mcp_json(&home.join(".claude.json"));
        let settings = home.join(".claude").join("settings.json");
        let hook = settings.exists()
            && std::fs::read_to_string(&settings)
                .map(|c| c.contains("claude-hook"))
                .unwrap_or(false);
        let active = mcp || hook;
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active,
            detail: if active {
                "arshy reachable via Claude Code".into()
            } else {
                "run: arshy integrate --agent claude-code".into()
            },
        }
    }
}

struct Cursor;
impl AgentIntegration for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }
    fn name(&self) -> &'static str {
        "Cursor"
    }
    fn mechanism(&self) -> &'static str {
        "MCP"
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".cursor").exists() || which("cursor")
    }
    fn integrate(&self, home: &Path, arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
        let path = home.join(".cursor").join("mcp.json");
        merge_mcp_json(&path, "arshy", arshy_bin, dry_run)
    }
    fn uninstall(&self, home: &Path, dry_run: bool) -> Result<Vec<String>> {
        let path = home.join(".cursor").join("mcp.json");
        remove_mcp_json(&path, "arshy", dry_run)
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, _cwd: &Path) -> AgentStatus {
        let path = home.join(".cursor").join("mcp.json");
        let active = has_mcp_json(&path);
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active,
            detail: if active {
                "arshy MCP registered".into()
            } else {
                "run: arshy integrate --agent cursor".into()
            },
        }
    }
}

struct VsCode;
impl AgentIntegration for VsCode {
    fn id(&self) -> &'static str {
        "vscode"
    }
    fn name(&self) -> &'static str {
        "VS Code"
    }
    fn mechanism(&self) -> &'static str {
        "MCP"
    }
    fn detect(&self, home: &Path) -> bool {
        which("code")
            || home.join(".vscode").exists()
            || home.join(".config").join("Code").exists()
            || home.join(".vscode-server").exists()
    }
    fn integrate(&self, home: &Path, arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
        let path = home.join(".vscode").join("mcp.json");
        merge_mcp_json(&path, "arshy", arshy_bin, dry_run)
    }
    fn uninstall(&self, home: &Path, dry_run: bool) -> Result<Vec<String>> {
        let path = home.join(".vscode").join("mcp.json");
        remove_mcp_json(&path, "arshy", dry_run)
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, _cwd: &Path) -> AgentStatus {
        let path = home.join(".vscode").join("mcp.json");
        let active = has_mcp_json(&path);
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active,
            detail: if active {
                "arshy MCP registered".into()
            } else {
                "run: arshy integrate --agent vscode".into()
            },
        }
    }
}

struct Antigravity;
impl AgentIntegration for Antigravity {
    fn id(&self) -> &'static str {
        "antigravity"
    }
    fn name(&self) -> &'static str {
        "Gemini CLI / Antigravity"
    }
    fn mechanism(&self) -> &'static str {
        "MCP"
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".gemini").exists() || which("gemini")
    }
    fn integrate(&self, home: &Path, arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
        let path = home.join(".gemini").join("config").join("mcp_config.json");
        merge_mcp_json(&path, "arshy", arshy_bin, dry_run)
    }
    fn uninstall(&self, home: &Path, dry_run: bool) -> Result<Vec<String>> {
        let path = home.join(".gemini").join("config").join("mcp_config.json");
        remove_mcp_json(&path, "arshy", dry_run)
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, _cwd: &Path) -> AgentStatus {
        let path = home.join(".gemini").join("config").join("mcp_config.json");
        let active = has_mcp_json(&path);
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active,
            detail: if active {
                "arshy MCP registered".into()
            } else {
                "run: arshy integrate --agent antigravity".into()
            },
        }
    }
}

/// Shared logic for agents that read an `AGENTS.md` instruction file.
fn agents_md_agent_integrate(cwd: &Path, dry_run: bool) -> Result<Vec<String>> {
    inject_agents_md(&cwd.join("AGENTS.md"), dry_run)
}
fn agents_md_agent_uninstall(cwd: &Path, dry_run: bool) -> Result<Vec<String>> {
    remove_agents_md(&cwd.join("AGENTS.md"), dry_run)
}
fn agents_md_agent_active(cwd: &Path) -> bool {
    has_agents_md(&cwd.join("AGENTS.md"))
}

struct Codex;
impl AgentIntegration for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn name(&self) -> &'static str {
        "OpenAI Codex"
    }
    fn mechanism(&self) -> &'static str {
        "MCP + AGENTS.md"
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".codex").exists() || which("codex")
    }
    fn integrate(&self, home: &Path, arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
        let mut actions = merge_codex_config(&codex_config_path(home), arshy_bin, dry_run)?;
        let cwd = std::env::current_dir().unwrap_or_default();
        actions.extend(agents_md_agent_integrate(&cwd, dry_run)?);
        Ok(actions)
    }
    fn uninstall(&self, home: &Path, dry_run: bool) -> Result<Vec<String>> {
        let mut actions = remove_codex_config(&codex_config_path(home), dry_run)?;
        let cwd = std::env::current_dir().unwrap_or_default();
        actions.extend(agents_md_agent_uninstall(&cwd, dry_run)?);
        Ok(actions)
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, cwd: &Path) -> AgentStatus {
        let mcp = has_codex_mcp(&codex_config_path(home));
        let agents_md = agents_md_agent_active(cwd);
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active: mcp || agents_md,
            detail: if mcp && agents_md {
                "MCP registered + AGENTS.md instructions".into()
            } else if mcp {
                "MCP registered; inject AGENTS.md instructions (arshy integrate --agent codex)"
                    .into()
            } else if agents_md {
                "AGENTS.md has instructions; register MCP (arshy integrate --agent codex)".into()
            } else {
                "run: arshy integrate --agent codex".into()
            },
        }
    }
}

struct OpenCode;
impl AgentIntegration for OpenCode {
    fn id(&self) -> &'static str {
        "opencode"
    }
    fn name(&self) -> &'static str {
        "OpenCode"
    }
    fn mechanism(&self) -> &'static str {
        "AGENTS.md"
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".config").join("opencode").exists() || which("opencode")
    }
    fn integrate(&self, _home: &Path, _arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
        let cwd = std::env::current_dir().unwrap_or_default();
        agents_md_agent_integrate(&cwd, dry_run)
    }
    fn uninstall(&self, _home: &Path, dry_run: bool) -> Result<Vec<String>> {
        let cwd = std::env::current_dir().unwrap_or_default();
        agents_md_agent_uninstall(&cwd, dry_run)
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, cwd: &Path) -> AgentStatus {
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active: agents_md_agent_active(cwd),
            detail: if agents_md_agent_active(cwd) {
                "AGENTS.md has arshy instructions".into()
            } else {
                "add arshy instructions to your project AGENTS.md".into()
            },
        }
    }
}

struct Aider;
impl AgentIntegration for Aider {
    fn id(&self) -> &'static str {
        "aider"
    }
    fn name(&self) -> &'static str {
        "Aider"
    }
    fn mechanism(&self) -> &'static str {
        "AGENTS.md"
    }
    fn detect(&self, home: &Path) -> bool {
        home.join(".aider.conf.yml").exists() || which("aider")
    }
    fn integrate(&self, _home: &Path, _arshy_bin: &Path, dry_run: bool) -> Result<Vec<String>> {
        let cwd = std::env::current_dir().unwrap_or_default();
        agents_md_agent_integrate(&cwd, dry_run)
    }
    fn uninstall(&self, _home: &Path, dry_run: bool) -> Result<Vec<String>> {
        let cwd = std::env::current_dir().unwrap_or_default();
        agents_md_agent_uninstall(&cwd, dry_run)
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, cwd: &Path) -> AgentStatus {
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active: agents_md_agent_active(cwd),
            detail: if agents_md_agent_active(cwd) {
                "AGENTS.md has arshy instructions".into()
            } else {
                "add arshy instructions to your project AGENTS.md".into()
            },
        }
    }
}

struct WorkBuddy;
impl AgentIntegration for WorkBuddy {
    fn id(&self) -> &'static str {
        "workbuddy"
    }
    fn name(&self) -> &'static str {
        "WorkBuddy"
    }
    fn mechanism(&self) -> &'static str {
        "GUI PATH"
    }
    fn detect(&self, home: &Path) -> bool {
        // WorkBuddy stores its config under ~/.workbuddy; treat its presence as detection.
        home.join(".workbuddy").exists() || std::env::var("WORKBUDDY").is_ok()
    }
    fn integrate(&self, _home: &Path, _arshy_bin: &Path, _dry_run: bool) -> Result<Vec<String>> {
        // No file mutation: coverage comes from the GUI PATH layer (L2). The agent
        // inherits `~/.arshy/bin` in PATH after a restart, so `bash` resolves to arshy.
        Ok(vec![
            "Relies on L2 GUI PATH layer (no config file changes).".into(),
            "Restart WorkBuddy so it inherits ~/.arshy/bin in PATH.".into(),
            "For deterministic routing, configure its Bash tool to use ~/.arshy/bin/bash if supported.".into(),
        ])
    }
    fn uninstall(&self, _home: &Path, _dry_run: bool) -> Result<Vec<String>> {
        Ok(vec![])
    }
    fn status(&self, home: &Path, _arshy_bin: &Path, _cwd: &Path) -> AgentStatus {
        let hook = crate::cli::shell_wrapper::is_hook_active();
        AgentStatus {
            id: self.id(),
            name: self.name().into(),
            mechanism: self.mechanism().into(),
            detected: self.detect(home),
            active: hook,
            detail: if hook {
                "shell hook active; restart WorkBuddy to inherit PATH".into()
            } else {
                "run: arshy integrate (then restart WorkBuddy)".into()
            },
        }
    }
}

// ── Orchestration ────────────────────────────────────────────────────────────

/// Resolve the arshy binary that shims / MCP should point at (prefer an installed
/// binary over the running debug build).
fn resolve_arshy_bin() -> PathBuf {
    crate::cli::shell_wrapper::resolve_arshy_bin()
}

/// Remove arshy from a single agent (zero-residue: only that agent's config is
/// touched; global shims / GUI PATH are left intact). Unknown ids error out.
pub fn uninstall_agent(agent_id: &str, dry_run: bool) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| arshy_lib::ArshyError::Other("HOME is not set".into()))?;
    uninstall_agent_at(&home, agent_id, dry_run)
}

/// Testable core of `uninstall_agent` with an explicit home directory.
pub fn uninstall_agent_at(home: &Path, agent_id: &str, dry_run: bool) -> Result<()> {
    let all = all_agents();
    let Some(a) = all.iter().find(|a| a.id() == agent_id) else {
        let ids: Vec<&str> = all.iter().map(|a| a.id()).collect();
        return Err(arshy_lib::ArshyError::Other(format!(
            "unknown agent `{agent_id}`; valid ids: {}",
            ids.join(", ")
        )));
    };
    for act in a.uninstall(home, dry_run)? {
        println!("  ✓ [{}] {}", a.name(), act);
    }
    Ok(())
}

/// Wire arshy into every detected agent (L0–L3). `agent` restricts to one id.
pub fn integrate_all(agent: Option<&str>, dry_run: bool) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| arshy_lib::ArshyError::Other("HOME is not set".into()))?;
    let arshy_bin = resolve_arshy_bin();

    // L2: GUI session PATH (so desktop apps inherit the shim).
    if agent.is_none() {
        let actions = setup_gui_path(dry_run)?;
        for a in &actions {
            println!("  • L2 {}", a);
        }
        // L0 + L1: shims + terminal PATH (reuse shell_wrapper).
        if dry_run {
            println!("  • L0/L1 would prepare shim symlinks + terminal PATH (dry-run, skipped)");
        } else {
            crate::cli::shell_wrapper::setup_shims(&arshy_bin)?;
            crate::cli::shell_wrapper::setup_terminal_path()?;
            println!("  • L0/L1 shim symlinks + terminal PATH prepared");
        }
    }

    // L3: per-agent first-class integration.
    for a in all_agents() {
        if let Some(name) = agent {
            if a.id() != name {
                continue;
            }
        }
        if a.detect(&home) {
            match a.integrate(&home, &arshy_bin, dry_run) {
                Ok(actions) => {
                    for act in actions {
                        println!("  ✓ [{}] {}", a.name(), act);
                    }
                }
                Err(e) => eprintln!("  ✗ [{}] {}", a.name(), e),
            }
        } else if agent == Some(a.id()) {
            println!("  ⚠ [{}] not detected, skipped", a.name());
        }
    }

    if dry_run {
        println!("\n(dry-run) no changes were made.");
    } else {
        println!("\nDone. Restart your IDE / terminal to activate arshy.");
    }
    Ok(())
}

/// Reverse all L2/L3 integrations (L0/L1 shims are removed by `uninstall-hook`).
pub fn teardown_all(dry_run: bool) -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| arshy_lib::ArshyError::Other("HOME is not set".into()))?;

    let actions = teardown_gui_path(dry_run)?;
    for a in &actions {
        println!("  • L2 {}", a);
    }

    for a in all_agents() {
        if a.detect(&home) {
            match a.uninstall(&home, dry_run) {
                Ok(acts) => {
                    for act in acts {
                        println!("  • [{}] {}", a.name(), act);
                    }
                }
                Err(e) => eprintln!("  ✗ [{}] {}", a.name(), e),
            }
        }
    }
    Ok(())
}

/// Per-agent diagnostics for `doctor` (L4).
pub fn agent_statuses() -> Vec<AgentStatus> {
    let home = dirs::home_dir().unwrap_or_default();
    let arshy_bin = resolve_arshy_bin();
    let cwd = std::env::current_dir().unwrap_or_default();
    all_agents().iter().map(|a| a.status(&home, &arshy_bin, &cwd)).collect()
}

/// Print the per-agent integration table (used by `arshy integrate --status`).
pub fn print_agent_status() -> Result<()> {
    println!("arshy agent integrations\n");
    println!("{:<14} {:<22} {:<14} NOTE", "AGENT", "MECHANISM", "STATUS");
    println!("{}", "─".repeat(70));
    for s in agent_statuses() {
        let status = if s.active {
            "active"
        } else if s.detected {
            "inactive"
        } else {
            "not found"
        };
        println!("{:<14} {:<22} {:<14} {}", s.id, s.mechanism, status, s.detail);
    }
    println!("\nRun `arshy integrate` to wire up all detected agents.");
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::io::Write;

    fn tmp_home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_prepend_path_adds_and_dedupes() {
        assert_eq!(
            prepend_path("/usr/bin:/bin", "/Users/x/.arshy/bin"),
            "/Users/x/.arshy/bin:/usr/bin:/bin"
        );
        // existing occurrence is removed, then re-added at front
        assert_eq!(
            prepend_path("/Users/x/.arshy/bin:/usr/bin:/bin", "/Users/x/.arshy/bin"),
            "/Users/x/.arshy/bin:/usr/bin:/bin"
        );
        // empty current -> just the prepend
        assert_eq!(prepend_path("", "/a/.arshy/bin"), "/a/.arshy/bin");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_gui_plist_xml_contains_dir() {
        let xml = gui_plist_xml(Path::new("/home/u/.arshy/bin"));
        assert!(xml.contains("/home/u/.arshy/bin"));
        assert!(xml.contains("com.arshy.path"));
        assert!(xml.contains("launchctl setenv PATH"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_gui_envd_content() {
        let c = gui_envd_content(Path::new("/home/u/.arshy/bin"));
        assert_eq!(c, "/home/u/.arshy/bin:$PATH\n");
    }

    #[test]
    fn test_merge_mcp_json_idempotent() {
        let tmp = tmp_home();
        let path = tmp.path().join(".cursor").join("mcp.json");
        let bin = PathBuf::from("/usr/local/bin/arshy");

        let a1 = merge_mcp_json(&path, "arshy", &bin, false).unwrap();
        assert!(a1.iter().any(|s| s.contains("→")));
        assert!(path.exists());

        // second run must be a no-op (already configured)
        let a2 = merge_mcp_json(&path, "arshy", &bin, false).unwrap();
        assert!(a2.iter().any(|s| s.contains("already configured")));

        // verify JSON shape
        let data: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(data["mcpServers"]["arshy"]["command"], "/usr/local/bin/arshy");
        assert_eq!(data["mcpServers"]["arshy"]["args"][0], "--from-mcp");

        // remove — zero residue: the file only held the arshy entry, so it is deleted
        let r = remove_mcp_json(&path, "arshy", false).unwrap();
        assert!(!r.is_empty());
        assert!(!path.exists(), "file must be deleted when it only held the arshy entry");
    }

    #[test]
    fn test_merge_mcp_json_preserves_other_servers() {
        let tmp = tmp_home();
        let path = tmp.path().join(".cursor").join("mcp.json");
        write_file(&path, r#"{"mcpServers":{"other":{"command":"other-bin","args":[]}}}"#);
        let bin = PathBuf::from("/usr/local/bin/arshy");
        merge_mcp_json(&path, "arshy", &bin, false).unwrap();
        let data: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(data["mcpServers"]["other"].is_object());
        assert!(data["mcpServers"]["arshy"].is_object());
    }

    #[test]
    fn test_inject_agents_md_idempotent() {
        let tmp = tmp_home();
        let path = tmp.path().join("AGENTS.md");
        write_file(&path, "# Project\n");

        let a1 = inject_agents_md(&path, false).unwrap();
        assert!(a1.iter().any(|s| s.contains("injected")));
        let a2 = inject_agents_md(&path, false).unwrap();
        assert!(a2.iter().any(|s| s.contains("already present")));

        assert!(has_agents_md(&path));
        // marker appears exactly twice (open + close)
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.matches(ARSHY_AGENTS_MD_MARKER).count(), 2);

        // removal strips the section
        let r = remove_agents_md(&path, false).unwrap();
        assert!(!r.is_empty());
        assert!(!has_agents_md(&path));
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("# Project"));
    }

    #[test]
    fn test_claude_detect_and_status() {
        let tmp = tmp_home();
        // create .claude.json with arshy mcp -> detected + active
        write_file(
            &tmp.path().join(".claude.json"),
            r#"{"mcpServers":{"arshy":{"command":"arshy","args":["--from-mcp"]}}}"#,
        );
        let c = ClaudeCode;
        assert!(c.detect(tmp.path()));
        let st = c.status(tmp.path(), &PathBuf::from("/bin/arshy"), tmp.path());
        assert!(st.detected);
        assert!(st.active);
    }

    #[test]
    fn test_cursor_integrate_uninstall_roundtrip() {
        let tmp = tmp_home();
        let c = Cursor;
        assert!(!c.detect(tmp.path()));
        write_file(&tmp.path().join(".cursor").join("x"), "");
        assert!(c.detect(tmp.path()));

        let bin = PathBuf::from("/usr/local/bin/arshy");
        let acts = c.integrate(tmp.path(), &bin, false).unwrap();
        assert!(acts.iter().any(|s| s.contains("→")));
        assert!(has_mcp_json(&tmp.path().join(".cursor").join("mcp.json")));

        c.uninstall(tmp.path(), false).unwrap();
        assert!(!has_mcp_json(&tmp.path().join(".cursor").join("mcp.json")));
    }

    #[test]
    fn test_workbuddy_detect() {
        let tmp = tmp_home();
        // detection relies on ~/.workbuddy presence in real HOME; in test HOME it's absent
        let w = WorkBuddy;
        assert!(!w.detect(tmp.path()));
        // with WORKBUDDY env set, detected
        std::env::set_var("WORKBUDDY", "1");
        assert!(w.detect(tmp.path()));
        std::env::remove_var("WORKBUDDY");
    }

    #[test]
    fn test_all_agents_registry_nonempty() {
        assert!(!all_agents().is_empty());
    }
    #[test]
    fn test_codex_config_merge_preserves_existing() {
        let tmp = tmp_home();
        let path = codex_config_path(tmp.path());
        write_file(
            &path,
            "# my settings\nmodel = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"npx\"\nargs = [\"-y\", \"@foo/bar\"]\n",
        );
        let bin = PathBuf::from("/usr/local/bin/arshy");
        let acts = merge_codex_config(&path, &bin, false).unwrap();
        assert!(acts.iter().any(|s| s.contains("→")), "{:?}", acts);
        assert!(has_codex_mcp(&path));

        let after = std::fs::read_to_string(&path).unwrap();
        // Original content untouched
        assert!(after.contains("model = \"gpt-5\""), "original settings lost");
        assert!(after.contains("[mcp_servers.other]"), "other MCP server lost");
        assert!(after.contains("# my settings"), "comments lost");
        // Block present with portable command
        assert!(after.contains("[mcp_servers.arshy]"), "{after}");
        assert!(after.contains("command = \"/usr/local/bin/arshy\""));
        assert!(after.contains("--from-mcp"));
    }

    #[test]
    fn test_codex_config_idempotent() {
        let tmp = tmp_home();
        let path = codex_config_path(tmp.path());
        write_file(&path, "model = \"gpt-5\"\n");
        let bin = PathBuf::from("/usr/local/bin/arshy");
        merge_codex_config(&path, &bin, false).unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let acts = merge_codex_config(&path, &bin, false).unwrap();
        assert!(acts.iter().any(|s| s.contains("already")), "{:?}", acts);
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after, "re-run must not modify the file");
    }

    #[test]
    fn test_codex_config_remove() {
        let tmp = tmp_home();
        let path = codex_config_path(tmp.path());
        write_file(
            &path,
            "# comment\nmodel = \"gpt-5\"\n\n[mcp_servers.arshy]\ncommand = \"/usr/local/bin/arshy\"\nargs = [\"--from-mcp\"]\n\n[mcp_servers.other]\ncommand = \"npx\"\n",
        );
        let acts = remove_codex_config(&path, false).unwrap();
        assert!(acts.iter().any(|s| s.contains("removed")), "{:?}", acts);
        assert!(!has_codex_mcp(&path));
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("arshy"), "block not fully removed: {after}");
        assert!(after.contains("model = \"gpt-5\""), "settings lost: {after}");
        assert!(after.contains("[mcp_servers.other]"), "other server lost: {after}");
        assert!(after.contains("# comment"), "comment lost: {after}");
    }

    #[test]
    #[serial]
    fn test_codex_integrate_uninstall_roundtrip() {
        let tmp = tmp_home();
        let c = Codex;
        // detection also consults PATH (which("codex")) — don't assert on absence

        // chdir into the temp dir so AGENTS.md injection stays inside the sandbox
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = (|| {
            let bin = PathBuf::from("/usr/local/bin/arshy");
            let acts = c.integrate(tmp.path(), &bin, false)?;
            assert!(acts.iter().any(|s| s.contains("Codex MCP server")), "{acts:?}");
            assert!(has_codex_mcp(&codex_config_path(tmp.path())));
            assert!(has_agents_md(&tmp.path().join("AGENTS.md")));

            c.uninstall(tmp.path(), false)?;
            assert!(!has_codex_mcp(&codex_config_path(tmp.path())));
            assert!(!has_agents_md(&tmp.path().join("AGENTS.md")));
            Ok::<(), arshy_lib::ArshyError>(())
        })();
        std::env::set_current_dir(old_cwd).unwrap();
        result.unwrap();
    }
    #[test]
    #[serial]
    fn test_uninstall_agent_zero_residue_codex() {
        let tmp = tmp_home();
        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = (|| {
            let bin = PathBuf::from("/usr/local/bin/arshy");
            Codex.integrate(tmp.path(), &bin, false)?;
            let cfg = codex_config_path(tmp.path());
            let md = tmp.path().join("AGENTS.md");
            assert!(cfg.exists() && md.exists(), "setup should create both files");

            uninstall_agent_at(tmp.path(), "codex", false)?;
            assert!(!cfg.exists(), "codex config must be deleted (zero residue)");
            assert!(
                !md.exists(),
                "AGENTS.md must be deleted when it only held arshy (zero residue)"
            );
            Ok::<(), arshy_lib::ArshyError>(())
        })();
        std::env::set_current_dir(old_cwd).unwrap();
        result.unwrap();
    }

    #[test]
    #[serial]
    fn test_uninstall_agent_preserves_preexisting_config() {
        let tmp = tmp_home();
        let cfg = codex_config_path(tmp.path());
        write_file(&cfg, "model = \"gpt-5\"\n# user comment\n");
        let md = tmp.path().join("AGENTS.md");
        write_file(&md, "# My Project\n\nSome content\n");

        let old_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = (|| {
            let bin = PathBuf::from("/usr/local/bin/arshy");
            Codex.integrate(tmp.path(), &bin, false)?;
            uninstall_agent_at(tmp.path(), "codex", false)?;

            // Original user content survives, arshy traces gone
            let after = std::fs::read_to_string(&cfg).unwrap();
            assert!(after.contains("model = \"gpt-5\""), "user config lost: {after}");
            assert!(after.contains("# user comment"), "user comment lost: {after}");
            assert!(!after.contains("arshy"), "arshy residue in config: {after}");

            let md_after = std::fs::read_to_string(&md).unwrap();
            assert!(md_after.contains("# My Project"), "AGENTS.md content lost");
            assert!(!md_after.contains("arshy"), "arshy residue in AGENTS.md: {md_after}");
            Ok::<(), arshy_lib::ArshyError>(())
        })();
        std::env::set_current_dir(old_cwd).unwrap();
        result.unwrap();
    }

    #[test]
    fn test_uninstall_agent_unknown_id() {
        let tmp = tmp_home();
        let err = uninstall_agent_at(tmp.path(), "not-an-agent", true).unwrap_err();
        assert!(err.to_string().contains("unknown agent"), "{err}");
    }
}
