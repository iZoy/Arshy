//! Daemon lifecycle — PID file, stale socket cleanup, process detection.

use arshy_lib::config::xdg_data_home;
use arshy_lib::Result;
use std::path::{Path, PathBuf};

/// PID file path: `${XDG_DATA_HOME}/arshy/arshyd.pid`
fn pid_path() -> PathBuf {
    PathBuf::from(xdg_data_home()).join("arshy").join("arshyd.pid")
}

/// Write PID to the given file path.
fn write_pid_to(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, std::process::id().to_string())?;
    Ok(())
}

/// Check the PID file and return the stored PID (0 if not running or stale).
fn check_pid_file(path: &Path) -> Result<u32> {
    if !path.exists() {
        return Ok(0);
    }
    let content = std::fs::read_to_string(path)?;
    let pid: u32 = content.trim().parse().unwrap_or(0);
    if pid == 0 || !is_process_alive(pid) {
        let _ = std::fs::remove_file(path);
        return Ok(0);
    }
    Ok(pid)
}

/// Write the current process PID to the default PID file.
pub fn write_pid() -> Result<()> {
    write_pid_to(&pid_path())
}

/// Remove the PID file on shutdown.
pub fn remove_pid() {
    let _ = std::fs::remove_file(pid_path());
}

/// Check if a daemon is already running.
pub fn check_running() -> Result<u32> {
    check_pid_file(&pid_path())
}

/// Clean up a stale socket file if no process owns it.
pub fn cleanup_stale_socket(socket_path: &Path) -> bool {
    if !socket_path.exists() {
        return false;
    }
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => false,
        Err(_) => {
            let _ = std::fs::remove_file(socket_path);
            true
        }
    }
}

/// Check if a process with the given PID is alive.
fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    #[test]
    fn nonexistent_process_not_alive() {
        assert!(!is_process_alive(u32::MAX - 1));
    }

    #[test]
    fn pid_file_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pid_file = tmp.path().join("arshyd.pid");

        write_pid_to(&pid_file).unwrap();
        assert!(pid_file.exists());

        let pid = check_pid_file(&pid_file).unwrap();
        assert_eq!(pid, std::process::id());

        std::fs::remove_file(&pid_file).unwrap();
        assert!(!pid_file.exists());
    }

    #[test]
    fn pid_file_stale_cleaned_up() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pid_file = tmp.path().join("arshyd.pid");
        // Write a PID that doesn't exist
        std::fs::write(&pid_file, (u32::MAX - 1).to_string()).unwrap();
        let pid = check_pid_file(&pid_file).unwrap();
        assert_eq!(pid, 0);
        assert!(!pid_file.exists(), "stale PID file should be removed");
    }

    #[test]
    fn cleanup_no_socket() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sock = tmp.path().join("nonexistent.sock");
        assert!(!cleanup_stale_socket(&sock));
    }
}
