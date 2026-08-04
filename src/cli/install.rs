//! Install / uninstall — MCP registrations, IDE configs, and binary self-update.

use arshy_lib::config::Config;
use arshy_lib::Result;
use std::path::PathBuf;

use super::integrate;
use super::shell_wrapper;

/// `arshy self-update` — copy the running build (and its sibling `arshyd`) over the
/// installed binaries, so the machine always runs the same code as the repo.
///
/// - Default destination: `~/.local/bin` for dev builds (`target/…`), or the
///   current binary's own directory when it is already installed (no-op).
/// - `--dest <dir>` overrides the destination.
pub(crate) fn self_update(dest: Option<PathBuf>) -> Result<()> {
    let exe = std::env::current_exe()?;
    let arshyd = exe.parent().map(|p| p.join("arshyd")).filter(|p| p.is_file());
    let dest_dir = match dest {
        Some(d) => d,
        None => default_install_dir(&exe),
    };
    std::fs::create_dir_all(&dest_dir)?;

    let mut actions: Vec<String> = Vec::new();
    let mut copied: Vec<PathBuf> = Vec::new();

    let dest_arshy = dest_dir.join("arshy");
    if dest_arshy == exe {
        println!("arshy is already installed at {}", dest_arshy.display());
    } else {
        std::fs::copy(&exe, &dest_arshy)?;
        copied.push(dest_arshy.clone());
        actions.push(format!("updated {}", dest_arshy.display()));
    }

    if let Some(ad) = arshyd {
        let dest_arshyd = dest_dir.join("arshyd");
        if dest_arshyd != ad {
            std::fs::copy(&ad, &dest_arshyd)?;
            copied.push(dest_arshyd.clone());
            actions.push(format!("updated {}", dest_arshyd.display()));
        }
    }

    // Ensure both copies are executable (fs::copy preserves permissions, but
    // be explicit so a bare `arshy self-update` can never leave a non-executable
    // install behind).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in &copied {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    if actions.is_empty() {
        println!("arshy is already up to date at {}", dest_dir.display());
    } else {
        for a in &actions {
            println!("  ✓ {}", a);
        }
        println!("Restart the daemon to pick up the new binary: `arshy daemon restart`");
    }
    Ok(())
}

/// Default install directory for `self_update`:
/// - dev builds (path contains a `target` component) → `~/.local/bin`
/// - otherwise → the current binary's own directory (already installed)
pub(crate) fn default_install_dir(exe: &std::path::Path) -> PathBuf {
    let in_target = exe.components().any(|c| c.as_os_str() == "target");
    if in_target {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".local").join("bin")
    } else {
        exe.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Required permission entries for seamless arshy integration.
pub(crate) const ARSHY_PERMISSIONS: &[&str] = &[
    // MCP tools — auto-approve arshy_exec and arshy_query
    "mcp__arshy__arshy_exec",
    "mcp__arshy__arshy_query",
    // Bash tool — auto-approve arshy CLI commands
    "Bash(arshy *)",
    "Bash(arshyd *)",
];

pub(crate) fn install() -> Result<()> {
    // Historical entry point (Claude Code / Cursor only). The unified verb is
    // `arshy setup [<agent>]` — same idempotent, reversible behavior, all agents.
    eprintln!(
        "note: `arshy install` is deprecated — use `arshy setup <agent>` (or `arshy setup` for all)"
    );
    println!("Registering arshy as MCP server + permissions...");

    // Write MCP server to ~/.claude.json (Claude Code's actual config)
    let claude_json_path =
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".claude.json");
    install_mcp_in_claude_json(&claude_json_path)?;

    // Write MCP server to ~/.cursor/mcp.json (Cursor IDE)
    let cursor_mcp_path =
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".cursor").join("mcp.json");
    install_mcp_in_cursor(&cursor_mcp_path)?;

    // Write MCP server to ~/.gemini/config/mcp_config.json (Google Antigravity)
    let gemini_mcp_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("~"))
        .join(".gemini")
        .join("config")
        .join("mcp_config.json");
    install_mcp_in_antigravity(&gemini_mcp_path)?;

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
    } else if let Err(e) = crate::proxy::start_daemon() {
        println!("  ⚠ could not start daemon: {}", e);
        println!("    Start manually with: arshy daemon start");
    } else {
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

    println!();
    println!("Done! Restart your IDE to activate.");
    println!("  Claude Code:  {}", claude_json_path.display());
    println!("  Cursor:       {}", cursor_mcp_path.display());
    println!("  Antigravity:  {}", gemini_mcp_path.display());
    println!("  Permissions:  {}", settings_path.display());
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
                println!("  \u{2713} MCP server registered (user scope) in {}", path.display());
            } else {
                println!("  \u{2713} MCP server already configured in {}", path.display());
            }
        }
    }

    std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
    Ok(())
}

/// Write arshy MCP entry to ~/.cursor/mcp.json (Cursor IDE).
///
/// Cursor uses a `mcpServers` object at the top level, same schema as Claude Code.
fn install_mcp_in_cursor(path: &std::path::Path) -> Result<()> {
    let mut data: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Find the installed arshy binary (same logic as Claude Code)
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
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
                println!("  \u{2713} MCP server registered in {}", path.display());
            } else {
                println!("  \u{2713} MCP server already configured in {}", path.display());
            }
        }
    } else {
        // JSON root is not an object (e.g., null) — overwrite with a fresh config
        data = serde_json::json!({
            "mcpServers": {
                "arshy": arshy_entry
            }
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
        println!("  \u{2713} MCP server registered in {}", path.display());
    }

    Ok(())
}

/// Write arshy MCP entry to ~/.gemini/config/mcp_config.json (Google Antigravity).
fn install_mcp_in_antigravity(path: &std::path::Path) -> Result<()> {
    let mut data: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let arshy_bin = find_installed_binary("arshy")
        .or_else(|| {
            std::env::current_exe().ok().filter(|p| !p.to_string_lossy().contains("target"))
        })
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "arshy".into());

    let arshy_entry = serde_json::json!({
        "command": arshy_bin,
        "args": ["--from-mcp"]
    });

    if let Some(obj) = data.as_object_mut() {
        let servers = obj.entry("mcpServers").or_insert_with(|| serde_json::json!({}));
        if let Some(map) = servers.as_object_mut() {
            let changed = map.get("arshy").is_none()
                || map.get("arshy").and_then(|v| v.get("command"))
                    != Some(&serde_json::json!(arshy_bin.clone()));
            map.insert("arshy".into(), arshy_entry);
            if changed {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
                println!("  \u{2713} MCP server registered in {}", path.display());
            } else {
                println!("  \u{2713} MCP server already configured in {}", path.display());
            }
        }
    } else {
        data = serde_json::json!({
            "mcpServers": {
                "arshy": arshy_entry
            }
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&data)?)?;
        println!("  \u{2713} MCP server registered in {}", path.display());
    }

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

pub(crate) fn uninstall() -> Result<()> {
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

    // Remove from ~/.cursor/mcp.json (Cursor IDE)
    let cursor_mcp_path = home.join(".cursor").join("mcp.json");
    if cursor_mcp_path.exists() {
        let content = std::fs::read_to_string(&cursor_mcp_path)?;
        let mut data: serde_json::Value =
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        if let Some(obj) = data.as_object_mut() {
            if let Some(servers) = obj.get_mut("mcpServers") {
                if let Some(map) = servers.as_object_mut() {
                    map.remove("arshy");
                }
            }
        }
        std::fs::write(&cursor_mcp_path, serde_json::to_string_pretty(&data)?)?;
        println!("Removed arshy from {}", cursor_mcp_path.display());
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

    // Remove from ~/.gemini/config/mcp_config.json (Google Antigravity)
    let gemini_mcp_path = home.join(".gemini").join("config").join("mcp_config.json");
    if gemini_mcp_path.exists() {
        let content = std::fs::read_to_string(&gemini_mcp_path)?;
        let mut data: serde_json::Value =
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        if let Some(obj) = data.as_object_mut() {
            if let Some(servers) = obj.get_mut("mcpServers") {
                if let Some(map) = servers.as_object_mut() {
                    map.remove("arshy");
                }
            }
        }
        std::fs::write(&gemini_mcp_path, serde_json::to_string_pretty(&data)?)?;
        println!("Removed arshy from {}", gemini_mcp_path.display());
    }

    // Reverse the GUI PATH layer (L2) and any L3 per-agent integrations.
    let _ = integrate::teardown_all(false);
    println!("Reverted arshy GUI PATH layer and agent integrations.");

    // Remove the transparent shell shims (~/.arshy/bin + shell rc lines) —
    // previously these survived `arshy uninstall`, breaking the zero-residue
    // promise. User parsers in ~/.arshy/parsers are preserved.
    shell_wrapper::uninstall_hook()?;

    // Remove the installed binaries when running from the install dir
    // (~/.local/bin). Dev builds (target/…) and other locations are left
    // alone — the user owns those.
    let install_dir = home.join(".local").join("bin");
    if let Ok(exe) = std::env::current_exe() {
        if exe.parent() == Some(install_dir.as_path()) {
            let _ = std::fs::remove_file(install_dir.join("arshyd"));
            let _ = std::fs::remove_file(install_dir.join("arshy"));
            println!("Removed installed binaries from {}", install_dir.display());
        }
    }

    println!(
        "Task/event data is kept at {} — remove that directory manually for a full purge.",
        home.join(".local").join("share").join("arshy").display()
    );
    Ok(())
}

fn find_installed_binary(name: &str) -> Option<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let path = PathBuf::from(dir).join(name);
        if path.exists() && !path.to_string_lossy().contains("target") {
            return Some(path);
        }
    }
    None
}
