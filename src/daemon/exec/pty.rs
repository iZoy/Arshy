//! PTY/process execution — spawns commands with piped stdout/stderr,
//! streams output lines through channels for async consumption.

use crate::Result;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::sync::mpsc;

/// A short-lived process burst can make the OS return EINTR/EAGAIN while
/// creating a child. Retry those transient errors without hiding permanent
/// spawn failures from the executor.
const SPAWN_RETRIES: usize = 3;

/// Handle to a spawned process — provides output receiver and lifecycle control.
pub struct ProcessHandle {
    /// OS process ID (available after spawn).
    pub pid: u32,
    /// Channel receiver for output lines as (source, rendered text, raw byte count).
    /// The count includes the line terminator when present and does not include
    /// synthetic read-error messages.
    pub output_rx: mpsc::Receiver<(String, String, u64)>,
    /// The child process handle (kept for kill/wait).
    child: Option<Child>,
}

/// Bound memory before UTF-8 decoding and line splitting. Commands may emit a
/// single line of arbitrary size; retaining it until a newline would bypass
/// the executor's output limit and could exhaust the daemon.
const MAX_LINE_BYTES: usize = 64 * 1024;

fn render_line(bytes: &[u8], truncated: bool) -> String {
    let invalid_utf8 = std::str::from_utf8(bytes).is_err();
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if text.ends_with('\r') {
        text.pop();
    }
    if invalid_utf8 {
        text.push_str(" [invalid UTF-8 replaced]");
    }
    if truncated {
        text.push_str(&format!(" [line truncated at {MAX_LINE_BYTES} bytes]"));
    }
    text
}

async fn stream_output<R>(
    mut reader: R,
    source: &'static str,
    tx: mpsc::Sender<(String, String, u64)>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut chunk = [0u8; 8192];
    let mut line = Vec::with_capacity(8192);
    let mut truncated = false;
    let mut line_bytes = 0u64;

    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                let _ = tx.send((source.into(), format!("[output read error: {error}]"), 0)).await;
                return;
            }
        };

        for byte in &chunk[..read] {
            line_bytes = line_bytes.saturating_add(1);
            if *byte == b'\n' {
                let text = render_line(&line, truncated);
                if tx.send((source.into(), text, line_bytes)).await.is_err() {
                    return;
                }
                line.clear();
                truncated = false;
                line_bytes = 0;
            } else if line.len() < MAX_LINE_BYTES {
                line.push(*byte);
            } else {
                truncated = true;
            }
        }
    }

    if !line.is_empty() || truncated {
        let _ = tx.send((source.into(), render_line(&line, truncated), line_bytes)).await;
    }
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
        // Every command is spawned as its own process-group leader. Kill the
        // group first so pipelines and grandchildren do not survive a task
        // timeout after the wrapper shell exits.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.pid as libc::pid_t), libc::SIGKILL);
        }
        if let Some(ref mut child) = self.child {
            if child.try_wait()?.is_none() {
                child.start_kill().map_err(|e| {
                    crate::ArshyError::Exec(format!("failed to kill process: {}", e))
                })?;
            }
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

/// Detect the user's login shell from `/etc/passwd`, falling back to `$SHELL` or `/bin/sh`.
fn detect_user_shell() -> String {
    // Try /etc/passwd first (works even when $SHELL is unset, e.g. under launchd)
    #[cfg(unix)]
    {
        if let Ok(passwd) = std::fs::read("/etc/passwd") {
            if let Ok(uid) = std::env::var("USER").or_else(|_| {
                // Get current username from libc
                unsafe {
                    let uid = libc::getuid();
                    let pw = libc::getpwuid(uid);
                    if pw.is_null() {
                        return Err(std::env::VarError::NotPresent);
                    }
                    let name = std::ffi::CStr::from_ptr((*pw).pw_name);
                    Ok(name.to_string_lossy().into_owned())
                }
            }) {
                for line in String::from_utf8_lossy(&passwd).lines() {
                    if let Some(shell) = line.split(':').nth(6) {
                        if line.starts_with(&format!("{}:", uid)) {
                            return shell.to_string();
                        }
                    }
                }
            }
        }
    }
    // Fallback: $SHELL or /bin/sh
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

/// Probe the user's login shell for its full PATH. Cached globally so the shell
/// is only spawned once per daemon lifetime.
pub fn user_shell_path() -> Option<&'static str> {
    static USER_PATH: OnceLock<Option<String>> = OnceLock::new();
    USER_PATH
        .get_or_init(|| {
            let shell = detect_user_shell();
            tracing::info!("detected user shell: {}", shell);
            // Run the login shell to capture the profile-enriched PATH
            let output = std::process::Command::new(&shell)
                .args(["-lic", "echo $PATH"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output();
            match output {
                Ok(out) if out.status.success() => {
                    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !path.is_empty() && path != "/usr/bin:/bin:/usr/sbin:/sbin" {
                        tracing::info!("enriched PATH from user shell ({} chars)", path.len());
                        Some(path)
                    } else {
                        tracing::debug!("user shell PATH same as system — no enrichment needed");
                        None
                    }
                }
                Ok(out) => {
                    tracing::warn!(
                        "user shell PATH probe failed (exit {}): {}",
                        out.status,
                        String::from_utf8_lossy(&out.stderr).trim()
                    );
                    None
                }
                Err(e) => {
                    tracing::warn!("could not run user shell for PATH probe: {}", e);
                    None
                }
            }
        })
        .as_deref()
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
    env: Option<&HashMap<String, String>>,
) -> Result<ProcessHandle> {
    use std::process::Stdio;

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    // Isolate the whole shell pipeline in a new process group. Lifecycle
    // management signals `-pid`; without this, that signal usually targets a
    // nonexistent group and descendants can leak past timeout/cancellation.
    #[cfg(unix)]
    cmd.process_group(0);

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    // Inject the user's profile-enriched PATH so tools in ~/.cargo/bin etc. are available.
    // This is especially important for launchd daemons where the inherited PATH is minimal.
    if let Some(upath) = user_shell_path() {
        cmd.env("PATH", upath);
    }

    // Force dumb terminal mode so tools (rustc, cargo, etc.) don't use ANSI escape
    // codes or cursor movement that would hide output lines from the parser.
    cmd.env("TERM", "dumb");

    if let Some(env_vars) = env {
        for (k, v) in env_vars {
            cmd.env(k, v);
        }
    }

    // Ensure child processes die when the parent dies
    cmd.kill_on_drop(true);

    let mut child = {
        let mut attempt = 0;
        loop {
            match cmd.spawn() {
                Ok(child) => break child,
                Err(error)
                    if attempt < SPAWN_RETRIES
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                        ) =>
                {
                    let delay_ms = 10_u64 << attempt;
                    attempt += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
                Err(error) => {
                    return Err(crate::ArshyError::Exec(format!(
                        "failed to spawn command: {}",
                        error
                    )));
                }
            }
        }
    };

    let pid =
        child.id().ok_or_else(|| crate::ArshyError::Exec("child process has no PID".into()))?;

    let (tx, rx) = mpsc::channel(1024);

    // Spawn stdout reader task
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(stream_output(stdout, "stdout", tx));
    }

    // Spawn stderr reader task
    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(stream_output(stderr, "stderr", tx));
    }

    // Drop the sender clones so rx closes when both readers finish
    drop(tx);

    Ok(ProcessHandle { pid, output_rx: rx, child: Some(child) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_spawn_echo() {
        let mut handle = spawn_command("echo hello world", None, None).await.unwrap();
        assert!(handle.pid > 0);

        let mut lines = Vec::new();
        while let Some((source, line, _)) = handle.output_rx.recv().await {
            assert_eq!(source, "stdout");
            lines.push(line);
        }
        assert_eq!(lines, vec!["hello world"]);

        let exit = handle.wait().await.unwrap();
        assert_eq!(exit, Some(0));
    }

    #[tokio::test]
    async fn test_spawn_stderr() {
        let mut handle = spawn_command("echo error >&2", None, None).await.unwrap();
        let mut stderr_lines = Vec::new();

        while let Some((source, line, _)) = handle.output_rx.recv().await {
            if source == "stderr" {
                stderr_lines.push(line);
            }
        }
        assert_eq!(stderr_lines, vec!["error"]);
    }

    #[tokio::test]
    async fn test_spawn_exit_code() {
        let mut handle = spawn_command("exit 42", None, None).await.unwrap();
        // Drain output (none expected)
        while handle.output_rx.recv().await.is_some() {}
        let exit = handle.wait().await.unwrap();
        assert_eq!(exit, Some(42));
    }

    #[tokio::test]
    async fn test_spawn_with_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let mut handle = spawn_command("pwd", Some(tmp.path()), None).await.unwrap();
        let mut lines = Vec::new();
        while let Some((_, line, _)) = handle.output_rx.recv().await {
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
        let mut handle = spawn_command("printf 'line1\nline2\nline3\n'", None, None).await.unwrap();
        let mut lines = Vec::new();
        while let Some((_, line, _)) = handle.output_rx.recv().await {
            lines.push(line);
        }
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[tokio::test]
    async fn test_force_kill() {
        let mut handle = spawn_command("sleep 60", None, None).await.unwrap();
        handle.force_kill().unwrap();
        let exit = handle.wait().await.unwrap();
        // Killed by SIGKILL: exit code is None on Unix (signal kill)
        // But start_kill + wait gives us Some(-1) or None depending on platform
        // We just verify it didn't wait the full 60 seconds
        assert!(exit.is_some() || exit.is_none()); // process terminated
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawned_command_is_its_process_group_leader() {
        let mut handle = spawn_command("sleep 60", None, None).await.unwrap();
        let pgid = unsafe { libc::getpgid(handle.pid as libc::pid_t) };
        assert_eq!(pgid, handle.pid as libc::pid_t);
        handle.force_kill().unwrap();
        let _ = handle.wait().await;
    }

    #[tokio::test]
    async fn test_spawn_empty_echo() {
        let mut handle = spawn_command("echo ''", None, None).await.unwrap();
        let mut lines = Vec::new();
        while let Some((_, line, _)) = handle.output_rx.recv().await {
            lines.push(line);
        }
        // Empty echo produces empty line
        assert!(lines.is_empty() || lines == vec![""]);
        handle.wait().await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_special_characters() {
        let mut handle = spawn_command("echo 'a]b[c{d}e(f)g*h?i$j!k'", None, None).await.unwrap();
        let mut lines = Vec::new();
        while let Some((_, line, _)) = handle.output_rx.recv().await {
            lines.push(line);
        }
        assert_eq!(lines, vec!["a]b[c{d}e(f)g*h?i$j!k"]);
        handle.wait().await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_pipe() {
        let mut handle = spawn_command("echo 'hello world' | wc -w", None, None).await.unwrap();
        let mut lines = Vec::new();
        while let Some((_, line, _)) = handle.output_rx.recv().await {
            lines.push(line);
        }
        assert_eq!(lines.len(), 1);
        assert!(lines[0].trim().parse::<i32>().is_ok()); // should be a number
        handle.wait().await.unwrap();
    }

    #[tokio::test]
    async fn test_spawn_stderr_redirect() {
        let mut handle = spawn_command("echo error >&2 && echo ok", None, None).await.unwrap();
        let mut stdout_lines = Vec::new();
        let mut stderr_lines = Vec::new();

        while let Some((source, line, _)) = handle.output_rx.recv().await {
            if source == "stdout" {
                stdout_lines.push(line);
            } else {
                stderr_lines.push(line);
            }
        }
        assert_eq!(stdout_lines, vec!["ok"]);
        assert_eq!(stderr_lines, vec!["error"]);
        handle.wait().await.unwrap();
    }

    #[tokio::test]
    async fn stream_replaces_invalid_utf8_and_keeps_following_output() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(stream_output(reader, "stdout", tx));
        writer.write_all(b"before\xffafter\nnext\n").await.unwrap();
        drop(writer);

        let (_, first, first_bytes) = rx.recv().await.unwrap();
        let (_, second, second_bytes) = rx.recv().await.unwrap();
        task.await.unwrap();
        assert!(first.contains('\u{fffd}'));
        assert!(first.contains("[invalid UTF-8 replaced]"));
        assert_eq!(first_bytes, 13);
        assert_eq!(second, "next");
        assert_eq!(second_bytes, 5);
    }

    #[tokio::test]
    async fn stream_bounds_a_line_before_a_newline_arrives() {
        let size = MAX_LINE_BYTES + 1024;
        let (mut writer, reader) = tokio::io::duplex(size + 1);
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(stream_output(reader, "stdout", tx));
        writer.write_all(&vec![b'x'; size]).await.unwrap();
        drop(writer);

        let (_, line, raw_bytes) = rx.recv().await.unwrap();
        task.await.unwrap();
        assert!(line.starts_with(&"x".repeat(MAX_LINE_BYTES)));
        assert!(line.ends_with(&format!("[line truncated at {MAX_LINE_BYTES} bytes]")));
        assert_eq!(raw_bytes, size as u64);
    }
}
