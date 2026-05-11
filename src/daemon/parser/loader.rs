use arshy_lib::Result;

/// File-system watcher for hot-reloading parser changes.
/// Uses `notify` crate in implementation phase.
#[allow(dead_code)]
pub struct ParserWatcher;

#[allow(dead_code)]
impl ParserWatcher {
    pub fn new(_dirs: &[std::path::PathBuf]) -> Result<Self> {
        Ok(Self)
    }
}
