//! Process manager — handles graceful termination with signal escalation.
//!
//! Kill strategy: SIGINT → wait grace_ms → SIGTERM → wait force_ms → SIGKILL.

use super::pty::ProcessHandle;
use crate::Result;

/// Gracefully terminate an entire process tree with escalating signals.
///
/// Steps (each targets the process group + direct PID):
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

    // Step 1: SIGINT to process group + direct PID
    send_signal_tree(pid, libc::SIGINT);
    tracing::debug!("sent SIGINT to pid {} and process group", pid);

    if wait_for_exit(handle, grace_ms).await? {
        tracing::debug!("pid {} exited after SIGINT", pid);
        return Ok(true);
    }

    // Step 2: SIGTERM to process group + direct PID
    send_signal_tree(pid, libc::SIGTERM);
    tracing::debug!("sent SIGTERM to pid {} and process group", pid);

    if wait_for_exit(handle, force_ms).await? {
        tracing::debug!("pid {} exited after SIGTERM", pid);
        return Ok(true);
    }

    // Step 3: SIGKILL to process group + direct PID
    tracing::warn!("force killing pid {} and its children", pid);
    send_signal_tree(pid, libc::SIGKILL);
    handle.force_kill()?;
    Ok(false)
}

/// Send a signal to a process tree: the process group (negative PID) and the
/// direct PID. This catches child processes spawned by shell pipelines and
/// build tools that fork subprocesses.
fn send_signal_tree(pid: u32, signal: libc::c_int) {
    let pgid = -(pid as libc::pid_t);
    // SAFETY: kill() with valid PID and standard signals (SIGINT, SIGTERM, SIGKILL).
    unsafe {
        libc::kill(pgid, signal);
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
