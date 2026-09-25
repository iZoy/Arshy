//! Daemon lifecycle — PID file, stale socket cleanup, process detection.

use crate::config::xdg_data_home;
use crate::Result;
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
    let pid_text = std::process::id().to_string();
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(pid_text.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read_to_string(path).unwrap_or_default();
            let pid = existing.trim().parse::<u32>().unwrap_or(0);
            if pid > 0 && is_process_alive(pid) {
                return Err(crate::ArshyError::Other(format!(
                    "daemon already running (pid {})",
                    pid
                )));
            }
            let _ = std::fs::remove_file(path);
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
            use std::io::Write;
            file.write_all(pid_text.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
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
/// No-op when ARSHY_TEST_NO_LIFECYCLE=1.
pub fn write_pid() -> Result<()> {
    if std::env::var("ARSHY_TEST_NO_LIFECYCLE").is_ok() {
        return Ok(());
    }
    write_pid_to(&pid_path())
}

/// Remove the PID file on shutdown.
/// No-op when ARSHY_TEST_NO_LIFECYCLE=1.
pub fn remove_pid() {
    if std::env::var("ARSHY_TEST_NO_LIFECYCLE").is_ok() {
        return;
    }
    let _ = std::fs::remove_file(pid_path());
}

/// Check if a daemon is already running.
/// Returns 0 (not running) when ARSHY_TEST_NO_LIFECYCLE=1 is set
/// (for integration tests that spawn multiple daemons).
pub fn check_running() -> Result<u32> {
    if std::env::var("ARSHY_TEST_NO_LIFECYCLE").is_ok() {
        return Ok(0);
    }
    check_pid_file(&pid_path())
}

/// Remove an abandoned Unix socket without unlinking a live listener or an
/// unrelated filesystem entry. Unknown connect errors are not proof that the
/// socket is stale, so startup fails closed and leaves the path untouched.
pub fn cleanup_stale_socket(socket_path: &Path) -> Result<()> {
    use std::os::unix::fs::FileTypeExt;

    let metadata = match std::fs::symlink_metadata(socket_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_socket() {
        return Err(crate::ArshyError::Other(format!(
            "refusing to replace non-socket path {}",
            socket_path.display()
        )));
    }

    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => Err(crate::ArshyError::Other(format!(
            "daemon socket is already accepting connections: {}",
            socket_path.display()
        ))),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            match std::fs::remove_file(socket_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(crate::ArshyError::Other(format!(
            "cannot determine whether daemon socket is stale ({}): {}",
            socket_path.display(),
            error
        ))),
    }
}

/// Check if a process with the given PID is alive.
fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Temporarily lower the process umask while binding the daemon socket so the
/// socket is owner-only from the instant the path becomes connectable.
///
/// UnixListener::bind starts listening before returning. Chmodding the path
/// afterwards leaves a short interval where a permissive process umask makes
/// the socket available to other local users. The restrictive umask also keeps
/// any concurrent startup-created files private; it is restored immediately.
struct RestrictiveUmask(libc::mode_t);

impl RestrictiveUmask {
    fn new() -> Self {
        // SAFETY: umask is process-global and accepts any mode mask. The guard
        // restores the previous value immediately after bind returns.
        Self(unsafe { libc::umask(0o077) })
    }
}

impl Drop for RestrictiveUmask {
    fn drop(&mut self) {
        // SAFETY: restores the valid mask returned by the matching umask call.
        unsafe { libc::umask(self.0) };
    }
}

/// Bind the daemon's Unix socket with owner-only access from creation onward.
pub fn bind_private_socket(path: &Path) -> Result<tokio::net::UnixListener> {
    let _umask = RestrictiveUmask::new();
    // Unix sockets are created with mode 0777 before applying umask; 0077
    // therefore makes the bound socket 0700 and blocks other UIDs immediately.
    // Keep the guard through bind so there is no chmod-after-bind exposure.
    Ok(tokio::net::UnixListener::bind(path)?)
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
        cleanup_stale_socket(&sock).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn create_stale_socket_in_helper_process() {
        let Some(path) = std::env::var_os("ARSHY_TEST_STALE_SOCKET_PATH") else {
            return;
        };
        let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
        drop(listener);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_removes_only_refused_socket_paths() {
        use std::os::unix::net::UnixListener;

        let tmp = tempfile::TempDir::new().unwrap();
        let stale = tmp.path().join("stale.sock");
        // Create the dead socket in a helper process and wait for process exit
        // so parallel tests cannot inherit a copy of its listening descriptor.
        let helper = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "daemon::lifecycle::tests::create_stale_socket_in_helper_process"])
            .env("ARSHY_TEST_STALE_SOCKET_PATH", &stale)
            .output()
            .unwrap();
        assert!(
            helper.status.success(),
            "stale socket helper failed: {}",
            String::from_utf8_lossy(&helper.stderr)
        );
        cleanup_stale_socket(&stale).unwrap();
        assert!(!stale.exists());

        let active = tmp.path().join("active.sock");
        let _listener = UnixListener::bind(&active).unwrap();
        assert!(cleanup_stale_socket(&active).is_err());
        assert!(active.exists(), "a live listener's socket path must be preserved");

        let regular_file = tmp.path().join("not-a-socket");
        std::fs::write(&regular_file, b"keep me").unwrap();
        assert!(cleanup_stale_socket(&regular_file).is_err());
        assert_eq!(std::fs::read(&regular_file).unwrap(), b"keep me");
    }

    #[tokio::test]
    async fn daemon_socket_is_owner_only_when_bind_returns() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("arshyd.sock");
        let _listener = bind_private_socket(&socket_path).unwrap();
        let mode = std::fs::metadata(socket_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
}
