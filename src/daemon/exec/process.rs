use arshy_lib::Result;

/// Process manager — tracks child processes and performs graceful termination.
///
/// Kill strategy: SIGINT → wait grace_ms → SIGTERM → wait force_ms → SIGKILL.
#[allow(dead_code)]
pub struct ProcessManager;

#[allow(dead_code)]
impl ProcessManager {
    pub fn new() -> Self { Self }

    /// Gracefully kill a process by pid, escalating through signals.
    pub async fn kill(_pid: u32, _grace_ms: u64, _force_ms: u64) -> Result<()> {
        Err(arshy_lib::ArshyError::Other("process kill not yet implemented".into()))
    }
}
