use arshy_lib::Result;

/// PTY spawn handle — will be wired to `portable-pty` in implementation phase.
#[allow(dead_code)]
pub struct PtyHandle {
    pub pid: u32,
}

/// Spawn a command in a PTY. Returns a handle for lifecycle management.
#[allow(dead_code)]
pub async fn spawn_pty(
    _command: &str,
    _cwd: Option<&std::path::Path>,
    _timeout_ms: Option<u64>,
) -> Result<PtyHandle> {
    Err(arshy_lib::ArshyError::Other("PTY execution not yet implemented".into()))
}
