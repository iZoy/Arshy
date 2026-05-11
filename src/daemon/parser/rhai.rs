use arshy_lib::ipc::TaskEvent;
use arshy_lib::Result;

/// Rhai script-based parser for stateful, cross-line parsing.
#[allow(dead_code)]
pub struct RhaiParser {
    pub name: String,
}

#[allow(dead_code)]
impl RhaiParser {
    /// Compile and load a Rhai parser script.
    pub fn from_script(_source: &str) -> Result<Self> {
        Ok(Self { name: "stub".into() })
    }

    /// Feed one line to the stateful parser.
    pub fn feed_line(&mut self, _line: &str) -> Vec<TaskEvent> {
        vec![]
    }

    /// Called on command completion — emit final events.
    pub fn on_complete(&mut self, _exit_code: i32) -> Vec<TaskEvent> {
        vec![]
    }
}
