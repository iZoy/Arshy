use arshy_lib::config::Config;
use arshy_lib::ipc::{self, Request, RunTaskParams, METHOD_RUN};
use arshy_lib::Result;
use std::io::Write;
use std::path::PathBuf;

/// Main entry point for shell wrapper interception.
/// Checks if it should intercept the shell command or transparently pass through.
pub fn run_wrapper(exe_name: &str, args: Vec<String>) -> Result<()> {
    let parent_name = get_parent_process_name();
    let intercept = should_intercept(&args, |key| std::env::var(key).is_ok(), parent_name);

    if intercept {
        let mut command_to_run = None;
        for (i, arg) in args.iter().enumerate() {
            if arg == "-c" {
                if let Some(cmd) = args.get(i + 1) {
                    command_to_run = Some(cmd.clone());
                }
                break;
            }
        }

        if let Some(cmd) = command_to_run {
            // Run synchronously via Tokio block_on (we are in main)
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
            rt.block_on(async {
                if let Err(e) = intercept_and_run(exe_name, &args, cmd).await {
                    // If interception fails, fallback to real shell
                    eprintln!("Arshy interception failed: {}. Falling back to real shell...", e);
                    let _ = execute_real_shell(exe_name, &args);
                }
            });
            Ok(())
        } else {
            execute_real_shell(exe_name, &args)
        }
    } else {
        execute_real_shell(exe_name, &args)
    }
}

/// Decides whether to intercept the shell execution based on command arguments,
/// environment variables, and parent process name.
fn should_intercept(
    args: &[String],
    env_check: impl Fn(&str) -> bool,
    parent_name: Option<String>,
) -> bool {
    let mut has_c = false;
    for arg in args {
        if arg == "-c" {
            has_c = true;
            break;
        }
    }
    if !has_c {
        return false;
    }

    let is_agent_env =
        env_check("ARSHY_AGENT") || env_check("CLAUDE_CODE") || env_check("CLINE_AGENT");

    let is_agent_parent = if let Some(parent) = parent_name {
        let name = parent.to_lowercase();
        name.contains("cursor")
            || name.contains("vscode")
            || name.contains("claude")
            || name.contains("agent")
            || name.contains("node")
            || name.contains("python")
            || name.contains("antigravity")
            || name.contains("gemini")
            || name.contains("agy")
    } else {
        false
    };

    is_agent_env || is_agent_parent
}

async fn intercept_and_run(_exe_name: &str, _args: &[String], command: String) -> Result<()> {
    let socket_path = {
        let cfg = Config::load(arshy_lib::config::CliOverrides::default()).unwrap_or_default();
        cfg.daemon.expanded_socket_path()
    };

    // Connect to daemon
    let mut daemon = ipc::connect(&socket_path).await?;

    let params = RunTaskParams {
        command,
        cwd: None,
        timeout_ms: None,
        mode: "auto".into(),
        parse_hint: None,
        env: None,
        errors_only: false,
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

    // Render using pretty renderer (stderr)
    crate::cli::render::render(&response.result);

    // Exit with the actual command exit code
    let exit_code = response.result.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0);
    std::process::exit(exit_code as i32);
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

pub fn install_hook() -> Result<()> {
    let home = std::env::var("HOME")
        .map_err(|_| arshy_lib::ArshyError::DaemonUnreachable("HOME env var not set".into()))?;
    let arshy_bin_dir = PathBuf::from(&home).join(".arshy").join("bin");
    let arshy_parsers_dir = PathBuf::from(&home).join(".arshy").join("parsers");

    // Create ~/.arshy/bin and ~/.arshy/parsers directories
    std::fs::create_dir_all(&arshy_bin_dir)?;
    std::fs::create_dir_all(&arshy_parsers_dir)?;

    let current_exe = std::env::current_exe()?;

    // Create symlinks
    let shells = ["sh", "bash", "zsh", "claude-hook"];
    for shell in &shells {
        let symlink_path = arshy_bin_dir.join(shell);
        if symlink_path.exists() {
            let _ = std::fs::remove_file(&symlink_path);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&current_exe, &symlink_path)?;
    }

    // Add to ~/.zshrc and ~/.bashrc
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

    // Also automatically install settings hook for Claude Code if installed
    let _ = install_claude_hook(&home, &arshy_bin_dir);

    println!("Shell hook successfully installed!");
    println!("Prepend directory: {}", arshy_bin_dir.display());
    println!(
        "Custom parser directory created: {} (drop your custom TOML rules here)",
        arshy_parsers_dir.display()
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

fn install_claude_hook(home: &str, arshy_bin_dir: &std::path::Path) -> Result<()> {
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

fn uninstall_claude_hook(home: &str) -> Result<()> {
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
/// rewrites it to use arshy run. Always outputs valid JSON to stdout.
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
                let rewritten = format!("arshy run --format pretty {}", escaped);
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

    #[test]
    fn test_should_intercept_interactive_no_c() {
        let args = vec!["sh".to_string(), "script.sh".to_string()];
        let env_check = |_key: &str| false;
        assert!(!should_intercept(&args, env_check, None));
    }

    #[test]
    fn test_should_intercept_non_interactive_no_agent() {
        let args = vec!["sh".to_string(), "-c".to_string(), "echo hello".to_string()];
        let env_check = |_key: &str| false;
        assert!(!should_intercept(&args, env_check, None));
        assert!(!should_intercept(&args, env_check, Some("bash".to_string())));
    }

    #[test]
    fn test_should_intercept_agent_env() {
        let args = vec!["sh".to_string(), "-c".to_string(), "cargo build".to_string()];

        let env_check_arshy = |key: &str| key == "ARSHY_AGENT";
        assert!(should_intercept(&args, env_check_arshy, None));

        let env_check_claude = |key: &str| key == "CLAUDE_CODE";
        assert!(should_intercept(&args, env_check_claude, None));
    }

    #[test]
    fn test_should_intercept_agent_parent() {
        let args = vec!["sh".to_string(), "-c".to_string(), "npm test".to_string()];
        let env_check = |_key: &str| false;

        assert!(should_intercept(&args, env_check, Some("Cursor".to_string())));
        assert!(should_intercept(
            &args,
            env_check,
            Some("/Applications/VSCode.app/Contents/MacOS/Electron".to_string())
        ));
        assert!(should_intercept(&args, env_check, Some("claude-code".to_string())));
        assert!(should_intercept(&args, env_check, Some("antigravity".to_string())));
        assert!(should_intercept(&args, env_check, Some("gemini-agent".to_string())));
        assert!(should_intercept(&args, env_check, Some("agy".to_string())));
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
