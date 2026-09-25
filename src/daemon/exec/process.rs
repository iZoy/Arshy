//! Process manager — handles graceful termination with signal escalation.
//!
//! Kill strategy: SIGINT → wait grace_ms → SIGTERM → wait force_ms → SIGKILL.

use super::pty::ProcessHandle;
use crate::Result;

/// Gracefully terminate an entire process tree with escalating signals.
///
/// Steps target the isolated process group, including the shell and descendants:
/// 1. Send SIGINT and wait `grace_ms` for exit
/// 2. Send SIGTERM and wait `force_ms` for exit
/// 3. Send SIGKILL (unconditional)
///
/// Returns Ok(true) if the process group exited before SIGKILL, Ok(false) if
/// SIGKILL escalation was required.
pub async fn graceful_kill(
    handle: &mut ProcessHandle,
    grace_ms: u64,
    force_ms: u64,
) -> Result<bool> {
    let pid = handle.pid;

    // Step 1: SIGINT to the process group.
    send_signal_tree(pid, libc::SIGINT);
    tracing::debug!("sent SIGINT to pid {} and process group", pid);

    if wait_for_group_exit(handle, grace_ms).await? {
        tracing::debug!("process group {} exited after SIGINT", pid);
        return Ok(true);
    }

    // Step 2: SIGTERM to the remaining process group.
    send_signal_tree(pid, libc::SIGTERM);
    tracing::debug!("sent SIGTERM to pid {} and process group", pid);

    if wait_for_group_exit(handle, force_ms).await? {
        tracing::debug!("process group {} exited after SIGTERM", pid);
        return Ok(true);
    }

    // Step 3: SIGKILL the remaining process group.
    tracing::warn!("force killing pid {} and its children", pid);
    send_signal_tree(pid, libc::SIGKILL);
    handle.force_kill()?;
    Ok(false)
}

/// Send a signal to the command's process group. The group leader may already
/// have exited while descendants remain, so signaling the old positive PID
/// could target an unrelated process if the OS reused it.
fn send_signal_tree(pid: u32, signal: libc::c_int) {
    let pgid = -(pid as libc::pid_t);
    // SAFETY: kill() with valid PID and standard signals (SIGINT, SIGTERM, SIGKILL).
    unsafe {
        libc::kill(pgid, signal);
    }
}

/// Poll until the whole process group exits, reaping the direct child along
/// the way. Waiting only for the shell leader can leak grandchildren that
/// ignore SIGINT after the shell exits.
async fn wait_for_group_exit(handle: &mut ProcessHandle, timeout_ms: u64) -> Result<bool> {
    let pid = handle.pid;
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    let poll_interval = tokio::time::Duration::from_millis(50);

    loop {
        let _ = handle.try_wait()?;
        if !process_group_exists(pid) {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(poll_interval).await;
    }
}

fn process_group_exists(pgid: u32) -> bool {
    // SAFETY: signal 0 only checks whether the process group exists.
    let result = unsafe { libc::kill(-(pgid as libc::pid_t), 0) };
    if result == 0 {
        true
    } else {
        // Only ESRCH proves absence. Treat other errors conservatively as
        // present so an unexpected OS error cannot prematurely end cleanup.
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn escalation_continues_after_shell_leader_exits() {
        let mut handle = super::super::pty::spawn_command(
            "sh -c 'trap \"\" INT; exec sleep 30' & echo $!",
            None,
            None,
        )
        .await
        .unwrap();
        let child_pid = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some((source, line, _)) = handle.output_rx.recv().await {
                    if source == "stdout" {
                        if let Ok(pid) = line.parse::<libc::pid_t>() {
                            break pid;
                        }
                    }
                }
            }
        })
        .await
        .expect("child PID was not written");

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while handle.try_wait().unwrap().is_none() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("shell leader did not exit before cancellation");

        assert!(process_group_exists(handle.pid));
        graceful_kill(&mut handle, 50, 500).await.unwrap();
        assert!(!process_group_exists(handle.pid));

        // Confirm the specifically launched child is gone as well.
        // SAFETY: signal 0 only checks whether the PID still exists.
        let child_exists = unsafe { libc::kill(child_pid, 0) } == 0;
        assert!(!child_exists, "child process {child_pid} survived cancellation");
    }
}
