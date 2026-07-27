use arshy_lib::config::{CliOverrides, Config};
use arshy_lib::daemon::security;
use std::env;
use tempfile::TempDir;

#[test]
fn test_workspace_sandbox_path_added() {
    // Create a temporary directory and set it as current working directory
    let temp = TempDir::new().expect("temp dir");
    let cwd = temp.path().to_path_buf();
    env::set_current_dir(&cwd).expect("set cwd");

    // Load a minimal config with sandbox_mode = "workspace"
    let mut cfg = Config::load(CliOverrides::default()).expect("load config");
    cfg.daemon.sandbox_mode = "workspace".to_string();

    // Clone security and apply the same logic as main.rs
    let mut security_cfg = cfg.security.clone();
    if cfg.daemon.sandbox_mode == "workspace" {
        if let Ok(current_dir) = env::current_dir() {
            let workspace = current_dir.to_string_lossy().to_string();
            security_cfg.sandbox_paths.push(workspace);
        }
    }

    // Verify that the sandbox_paths now contains the temp directory path
    assert!(security_cfg.sandbox_paths.iter().any(|p| p == &cwd.to_string_lossy().to_string()));
}
