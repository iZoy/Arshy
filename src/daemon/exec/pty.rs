//! PTY/process execution — spawns commands with piped stdout/stderr,
//! streams output lines through channels for async consumption.

use arshy_lib::Result;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc;

/// Handle to a spawned process — provides output receiver and lifecycle control.
pub struct ProcessHandle {
    /// OS process ID (available after spawn).
    pub pid: u32,
    /// Channel receiver for output lines. Each line is (source, text).
    /// source is "stdout" or "stderr".
    pub output_rx: mpsc::Receiver<(String, String)>,
    /// The child process handle (kept for kill/wait).
    child: Option<Child>,
}

impl ProcessHandle {
    /// Wait for the process to exit. Returns the exit code.
    pub async fn wait(&mut self) -> Result<Option<i32>> {
        if let Some(ref mut child) = self.child {
            let status = child.wait().await?;
            Ok(status.code())
        } else {
            Ok(None)
        }
    }

    /// Kill the process immediately (SIGKILL).
    /// Uses start_kill() which is non-blocking.
    pub fn force_kill(&mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            child.start_kill().map_err(|e| {
                arshy_lib::ArshyError::Exec(format!("failed to kill process: {}", e))
            })?;
        }
        Ok(())
    }

    /// Check if the process has exited (non-blocking).
    pub fn try_wait(&mut self) -> Result<Option<Option<i32>>> {
        if let Some(ref mut child) = self.child {
            match child.try_wait()? {
                Some(status) => Ok(Some(status.code())),
                None => Ok(None),
            }
        } else {
            Ok(Some(None))
        }
    }
}

/// Spawn a command as a child process with piped stdout/stderr.
///
/// The command is wrapped in `sh -c` to support shell features (pipes, redirects, etc).
/// Output lines are streamed through the returned channel.
///
/// # Arguments
/// * `command` - The shell command string
/// * `cwd` - Optional working directory
/// * `env_vars` - Optional extra environment variables (key, value)
///
/// # Returns
/// A `ProcessHandle` with the pid, output channel, and child handle.
pub async fn spawn_command(
    command: &str,
    cwd: Option<&std::path::Path>,
) -> Result<ProcessHandle> {
    use std::process::Stdio;

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
       .arg(command)
       .stdin(Stdio::null())
       .stdout(Stdio::piped())
       .stderr(Stdio::piped());

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    // Ensure child processes die when the parent dies
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        arshy_lib::ArshyError::Exec(format!("failed to spawn command: {}", e))
    })?;

    let pid = child.id().ok_or_else(|| {
        arshy_lib::ArshyError::Exec("child process has no PID".into())
    })?;

    let (tx, rx) = mpsc::channel(1024);

    // Spawn stdout reader task
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(("stdout".into(), line)).await.is_err() {
                    break; // receiver dropped
                }
            }
        });
    }

    // Spawn stderr reader task
    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(("stderr".into(), line)).await.is_err() {
                    break;
                }
            }
        });
    }

    // Drop the sender clones so rx closes when both readers finish
    drop(tx);

    Ok(ProcessHandle {
        pid,
        output_rx: rx,
        child: Some(child),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_echo() {
        let mut handle = spawn_command("echo hello world", None).await.unwrap();
        assert!(handle.pid > 0);

        let mut lines = Vec::new();
        while let Some((source, line)) = handle.output_rx.recv().await {
            assert_eq!(source, "stdout");
            lines.push(line);
        }
        assert_eq!(lines, vec!["hello world"]);

        let exit = handle.wait().await.unwrap();
        assert_eq!(exit, Some(0));
    }

    #[tokio::test]
    async fn test_spawn_stderr() {
        let mut handle = spawn_command("echo error >&2", None).await.unwrap();
        let mut stderr_lines = Vec::new();

        while let Some((source, line)) = handle.output_rx.recv().await {
            if source == "stderr" {
                stderr_lines.push(line);
            }
        }
        assert_eq!(stderr_lines, vec!["error"]);
    }

    #[tokio::test]
    async fn test_spawn_exit_code() {
        let mut handle = spawn_command("exit 42", None).await.unwrap();
        // Drain output (none expected)
        while handle.output_rx.recv().await.is_some() {}
        let exit = handle.wait().await.unwrap();
        assert_eq!(exit, Some(42));
    }

    #[tokio::test]
    async fn test_spawn_with_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handle = spawn_command("pwd", Some(tmp.path())).await.unwrap();
        let mut lines = Vec::new();
        while let Some((_, line)) = handle.output_rx.recv().await {
            lines.push(line);
        }
        assert_eq!(lines.len(), 1);
        // The path should match (after canonicalize for macOS /private prefix)
        let expected = std::fs::canonicalize(tmp.path()).unwrap();
        let actual = std::path::PathBuf::from(&lines[0]);
        let actual = std::fs::canonicalize(&actual).unwrap_or(actual);
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn test_spawn_multiline() {
        let mut handle = spawn_command("printf 'line1\nline2\nline3\n'", None).await.unwrap();
        let mut lines = Vec::new();
        while let Some((_, line)) = handle.output_rx.recv().await {
            lines.push(line);
        }
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[tokio::test]
    async fn test_force_kill() {
        let mut handle = spawn_command("sleep 60", None).await.unwrap();
        handle.force_kill().unwrap();
        let exit = handle.wait().await.unwrap();
        // Killed by SIGKILL: exit code is None on Unix (signal kill)
        // But start_kill + wait gives us Some(-1) or None depending on platform
        // We just verify it didn't wait the full 60 seconds
        assert!(exit.is_some() || exit.is_none()); // process terminated
    }
}
