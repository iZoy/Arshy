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
    /// When true, this pattern is deprecated and may be removed in a future version.
    pub deprecated: bool,
    /// Name of the replacement pattern, if any.
    pub replaced_by: Option<String>,
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
/// Detects common error/warning patterns across languages and toolchains.
pub fn raw_event(line: &str, seq: u64) -> TaskEvent {
    let severity = classify_severity(line);
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

/// Classify severity from a raw output line using common cross-language patterns.
fn classify_severity(line: &str) -> &'static str {
    let lower = line.to_lowercase();

    let is_error = lower.contains("error")
        || lower.contains("fatal")
        || lower.contains("failed")
        || lower.contains("panic")
        || lower.contains("aborted")
        || lower.contains("traceback")
        || lower.contains("killed")
        || lower.contains("segmentation fault")
        || lower.contains("bus error")
        || lower.contains("assertion failed")
        || lower.starts_with("e ")
        || lower.starts_with("e\t");

    if is_error {
        return "error";
    }

    let is_warning = lower.contains("warning")
        || lower.contains("warn")
        || lower.contains("deprecated")
        || lower.contains("notice")
        || lower.contains("attention")
        || lower.starts_with("w ")
        || lower.starts_with("w\t");

    if is_warning {
        return "warning";
    }

    "info"
}

/// Evaluate whether a stderr line looks like it contains an error, even when
/// the tool-specific parser didn't flag it. Used as a safety net for CLI tools
/// that write errors to stderr without structured formatting.
///
/// Returns true if the line strongly signals an error condition.
pub fn stderr_looks_like_error(line: &str) -> bool {
    let lower = line.to_lowercase();
    // Strong signals: explicit error keywords
    lower.contains("error:")
        || lower.contains("error ")
        || lower.contains("failed:")
        || lower.contains("fatal:")
        || lower.contains("panic:")
        || lower.contains("panic!")
        || lower.contains("traceback (most recent call last)")
        || lower.contains("segmentation fault")
        || lower.contains("abort trap")
        || lower.starts_with("e ")
        || lower.starts_with("e\t")
        // Common CLI patterns
        || lower.contains("command not found")
        || lower.contains("no such file")
        || lower.contains("cannot find")
        || lower.contains("permission denied")
        || lower.contains("access denied")
        || lower.contains("not found")
        || lower.contains("syntax error")
        || lower.contains("unexpected token")
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
            deprecated: false,
            replaced_by: None,
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
            deprecated: false,
            replaced_by: None,
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
            deprecated: false,
            replaced_by: None,
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

    /// S3: enhanced error/warning detection
    #[test]
    fn test_raw_error_keywords() {
        for line in &[
            "FAILED: build step",
            "fatal: unable to read config",
            "panic: runtime error: index out of range",
            "Aborted (core dumped)",
            "Killed: 9",
        ] {
            let event = raw_event(line, 0);
            assert_eq!(event.severity, Some("error".into()), "line: {}", line);
        }
    }

    #[test]
    fn test_raw_warning_keywords() {
        for line in &[
            "WARN: deprecated option",
            "warn: using fallback",
            "Deprecated: use --new-flag instead",
            "NOTICE: configuration changed",
            "attention: disk usage high",
        ] {
            let event = raw_event(line, 0);
            assert_eq!(event.severity, Some("warning".into()), "line: {}", line);
        }
    }

    #[test]
    fn test_raw_info_for_neutral_text() {
        for line in &[
            "building module",
            "compilation successful",
            "12 tests passed",
            "installed packages",
        ] {
            let event = raw_event(line, 0);
            assert_eq!(event.severity, Some("info".into()), "line: {}", line);
        }
    }

    #[test]
    fn test_stderr_looks_like_error_strong_signals() {
        assert!(stderr_looks_like_error("error: cannot find module 'fs'"));
        assert!(stderr_looks_like_error("Error: something went wrong"));
        assert!(stderr_looks_like_error("Command not found: arshy"));
        assert!(stderr_looks_like_error("Permission denied (os error 13)"));
        assert!(stderr_looks_like_error("No such file or directory"));
        assert!(stderr_looks_like_error("syntax error near unexpected token"));
    }

    #[test]
    fn test_stderr_looks_like_error_false_for_neutral() {
        assert!(!stderr_looks_like_error("building..."));
        assert!(!stderr_looks_like_error("100% complete"));
        assert!(!stderr_looks_like_error("compiling module"));
    }
}
