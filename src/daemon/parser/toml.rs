use arshy_lib::ipc::TaskEvent;
use arshy_lib::Result;

/// A TOML-based line-matching parser.
#[allow(dead_code)]
pub struct TomlParser {
    pub name: String,
}

#[allow(dead_code)]
impl TomlParser {
    /// Load parser definition from TOML content.
    pub fn from_toml(_content: &str) -> Result<Self> {
        Ok(Self { name: "stub".into() })
    }

    /// Parse one line of output. Returns Some(event) if matched.
    pub fn parse_line(&self, _line: &str) -> Option<TaskEvent> {
        None
    }
}

/// Raw fallback — every line becomes a log event.
pub fn raw_event(line: &str, seq: u64) -> TaskEvent {
    TaskEvent {
        seq,
        event_type: "log".into(),
        severity: Some("info".into()),
        code: None,
        message: line.to_string(),
        location: None,
        context: None,
    }
}
