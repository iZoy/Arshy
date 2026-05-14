//! Stateless line-by-line parser — matches each output line against regex patterns.
//!
//! Patterns are loaded from TOML definition files (see `toml_def.rs`).
//! This module only handles matching, not pattern loading.

use arshy_lib::ipc::{EventLocation, TaskEvent};
use regex::Regex;

/// A single regex pattern that matches a line and extracts event fields.
///
/// Fields are indexed by regex capture group number.
/// The `severity` field is fixed per pattern (not captured from output),
/// except when `fields.severity` references a capture group in the TOML def.
#[derive(Debug, Clone)]
pub struct LinePattern {
    pub regex: Regex,
    pub event_type: String,
    pub severity: String,
    pub file_group: Option<usize>,
    pub line_group: Option<usize>,
    pub col_group: Option<usize>,
    pub code_group: Option<usize>,
    pub message_group: Option<usize>,
}

/// A stateless line-matching parser.
///
/// Receives pre-compiled patterns at construction time (from the registry).
/// Each call to `parse_line()` tries patterns in order; first match wins.
pub struct TomlParser {
    patterns: Vec<LinePattern>,
}

impl TomlParser {
    /// Create a parser with pre-loaded patterns.
    pub fn new(patterns: Vec<LinePattern>) -> Self {
        Self { patterns }
    }

    /// Parse one line of output. Returns Some(event) if any pattern matched.
    pub fn parse_line(&self, line: &str) -> Option<TaskEvent> {
        for pat in &self.patterns {
            if let Some(caps) = pat.regex.captures(line) {
                let file = pat.file_group.and_then(|i| caps.get(i)).map(|m| m.as_str().to_string());
                let line_no = pat.line_group
                    .and_then(|i| caps.get(i))
                    .and_then(|m| m.as_str().parse::<u64>().ok());
                let col = pat.col_group
                    .and_then(|i| caps.get(i))
                    .and_then(|m| m.as_str().parse::<u64>().ok());
                let code = pat.code_group
                    .and_then(|i| caps.get(i))
                    .map(|m| m.as_str().to_string());
                let message = pat.message_group
                    .and_then(|i| caps.get(i))
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_else(|| line.to_string());

                let location = file.map(|f| EventLocation { file: f, line: line_no.unwrap_or(0), column: col });

                return Some(TaskEvent {
                    seq: 0, // caller sets seq
                    event_type: pat.event_type.clone(),
                    severity: Some(pat.severity.clone()),
                    code,
                    message,
                    location,
                    context: None,
                });
            }
        }
        None
    }
}

/// Raw fallback — every line becomes a log event.
///
/// Used when no parser matches the tool or when no pattern matches the line.
pub fn raw_event(line: &str, seq: u64) -> TaskEvent {
    let severity = if line.contains("error") || line.contains("Error") || line.contains("ERROR") || line.contains("fatal") {
        "error"
    } else if line.contains("warning") || line.contains("Warning") || line.contains("WARN") {
        "warning"
    } else {
        "info"
    };

    TaskEvent {
        seq,
        event_type: "log".into(),
        severity: Some(severity.into()),
        code: None,
        message: line.to_string(),
        location: None,
        context: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pattern(regex: &str, event_type: &str, severity: &str) -> LinePattern {
        LinePattern {
            regex: Regex::new(regex).unwrap(),
            event_type: event_type.into(),
            severity: severity.into(),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: None,
        }
    }

    #[test]
    fn test_basic_match() {
        let parser = TomlParser::new(vec![LinePattern {
            regex: Regex::new(r"^error: (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: Some(1),
        }]);
        let event = parser.parse_line("error: something broke");
        assert!(event.is_some());
        let e = event.unwrap();
        assert_eq!(e.severity, Some("error".into()));
        assert_eq!(e.message, "something broke");
    }

    #[test]
    fn test_no_match_returns_none() {
        let parser = TomlParser::new(vec![
            make_pattern(r"^error: (.+)$", "diagnostic", "error"),
        ]);
        assert!(parser.parse_line("all good").is_none());
    }

    #[test]
    fn test_empty_patterns() {
        let parser = TomlParser::new(vec![]);
        assert!(parser.parse_line("anything").is_none());
    }

    #[test]
    fn test_capture_groups() {
        let parser = TomlParser::new(vec![LinePattern {
            regex: Regex::new(r"^(.+?):(\d+): error: (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            file_group: Some(1),
            line_group: Some(2),
            col_group: None,
            code_group: None,
            message_group: Some(3),
        }]);
        let event = parser.parse_line("main.rs:42: error: undefined").unwrap();
        let loc = event.location.unwrap();
        assert_eq!(loc.file, "main.rs");
        assert_eq!(loc.line, 42);
        assert_eq!(event.message, "undefined");
    }

    #[test]
    fn test_first_match_wins() {
        let parser = TomlParser::new(vec![
            make_pattern(r"^error", "diagnostic", "error"),
            make_pattern(r"^error", "diagnostic", "warning"), // never reached
        ]);
        let event = parser.parse_line("error: x").unwrap();
        assert_eq!(event.severity, Some("error".into()));
    }

    #[test]
    fn test_raw_fallback() {
        let event = raw_event("hello world", 1);
        assert_eq!(event.event_type, "log");
        assert_eq!(event.severity, Some("info".into()));
        assert_eq!(event.message, "hello world");
    }

    #[test]
    fn test_raw_severity_detection() {
        let err = raw_event("Error: something failed", 1);
        assert_eq!(err.severity, Some("error".into()));

        let warn = raw_event("Warning: deprecated usage", 2);
        assert_eq!(warn.severity, Some("warning".into()));
    }
}
