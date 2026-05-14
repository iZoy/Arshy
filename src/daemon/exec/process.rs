//! Process manager — handles graceful termination with signal escalation.
//!
//! Kill strategy: SIGINT → wait grace_ms → SIGTERM → wait force_ms → SIGKILL.

use arshy_lib::Result;
use super::pty::ProcessHandle;

/// Gracefully terminate a process with escalating signals.
///
/// Steps:
/// 1. Send SIGINT and wait `grace_ms` for exit
/// 2. Send SIGTERM and wait `force_ms` for exit
/// 3. Send SIGKILL (unconditional)
///
/// Returns Ok(true) if process exited gracefully, Ok(false) if force-killed.
pub async fn graceful_kill(
    handle: &mut ProcessHandle,
    grace_ms: u64,
    force_ms: u64,
) -> Result<bool> {
    let pid = handle.pid;

    // Step 1: SIGINT (Ctrl+C equivalent)
    send_signal(pid, libc::SIGINT);
    tracing::debug!("sent SIGINT to pid {}", pid);

    // Wait for graceful exit
    if wait_for_exit(handle, grace_ms).await? {
        tracing::debug!("pid {} exited after SIGINT", pid);
        return Ok(true);
    }

    // Step 2: SIGTERM
    send_signal(pid, libc::SIGTERM);
    tracing::debug!("sent SIGTERM to pid {}", pid);

    if wait_for_exit(handle, force_ms).await? {
        tracing::debug!("pid {} exited after SIGTERM", pid);
        return Ok(true);
    }

    // Step 3: SIGKILL (no return)
    tracing::warn!("force killing pid {}", pid);
    handle.force_kill()?;
    Ok(false)
}

/// Send a signal to a process by PID.
/// Uses raw libc::kill on Unix. No-op on non-Unix (shouldn't happen for daemon).
fn send_signal(pid: u32, signal: libc::c_int) {
    // SAFETY: kill() is safe to call with a valid PID and signal.
    // We only use standard signals (SIGINT, SIGTERM, SIGKILL).
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

/// Poll the process for exit, sleeping in 50ms increments.
/// Returns Ok(true) if the process exited within timeout_ms, Ok(false) otherwise.
async fn wait_for_exit(handle: &mut ProcessHandle, timeout_ms: u64) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    let poll_interval = tokio::time::Duration::from_millis(50);

    loop {
        if let Some(_exit) = handle.try_wait()? {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(poll_interval).await;
    }
}

/// Terminate an entire process tree.
/// Sends the signal to the process group (negative PID) and the process directly.
#[allow(dead_code)] // future: process tree cleanup
pub fn kill_process_tree(pid: u32, signal: libc::c_int) {
    // Kill the entire process group by using negative PID.
    // libc::kill takes pid_t which is i32 on most platforms.
    let pgid = -(pid as i32);
    unsafe {
        libc::kill(pgid, signal);
        libc::kill(pid as libc::pid_t, signal);
    }
}
