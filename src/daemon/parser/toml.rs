//! Stateless line-by-line parser — matches each output line against regex patterns.
//!
//! Patterns are loaded from TOML definition files (see `toml_def.rs`).
//! This module only handles matching, not pattern loading.

use crate::ipc::{EventLocation, TaskEvent};
use regex::Regex;

/// A single regex pattern that matches a line and extracts event fields.
///
/// Fields are indexed by regex capture group number. `severity` is the fixed
/// fallback; `fields.severity` may point at a capture group whose text is
/// normalized to `error`/`warning`/`info` (unrecognized text falls back to
/// the fixed severity — fail-safe).
#[derive(Debug, Clone)]
pub struct LinePattern {
    pub regex: Regex,
    pub event_type: String,
    pub severity: String,
    pub severity_group: Option<usize>,
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
                let line_no = pat
                    .line_group
                    .and_then(|i| caps.get(i))
                    .and_then(|m| m.as_str().parse::<u64>().ok());
                let col = pat
                    .col_group
                    .and_then(|i| caps.get(i))
                    .and_then(|m| m.as_str().parse::<u64>().ok());
                let code = pat.code_group.and_then(|i| caps.get(i)).map(|m| m.as_str().to_string());
                let message = pat
                    .message_group
                    .and_then(|i| caps.get(i))
                    .map(|m| m.as_str().trim().to_string())
                    .unwrap_or_else(|| line.to_string());

                let location = file.map(|f| EventLocation {
                    file: f,
                    line: line_no.unwrap_or(0),
                    column: col,
                });

                // Dynamic severity: capture group text is normalized; any
                // unrecognized text falls back to the pattern's fixed value.
                let severity = pat
                    .severity_group
                    .and_then(|g| caps.get(g))
                    .and_then(|m| normalize_severity(m.as_str()))
                    .unwrap_or_else(|| pat.severity.clone());

                return Some(TaskEvent {
                    seq: 0, // caller sets seq
                    event_type: pat.event_type.clone(),
                    severity: Some(severity),
                    code,
                    message,
                    location,
                    context: None,
                    hint: None,
                });
            }
        }
        None
    }
}

/// Normalize a captured severity token to a canonical severity.
///
/// Returns `None` when the token is unrecognized so the caller falls back to
/// the pattern's fixed severity (fail-safe — never invent a severity).
pub fn normalize_severity(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("fatal") || lower.starts_with("error") {
        Some("error".into())
    } else if lower.starts_with("warn") {
        Some("warning".into())
    } else if lower.starts_with("info") || lower.starts_with("note") || lower.starts_with("debug") {
        Some("info".into())
    } else {
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
        hint: None,
    }
}

/// ASCII case-insensitive substring search. Avoids heap allocation from `to_lowercase()`.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack.as_bytes().windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// ASCII case-insensitive prefix check.
fn starts_with_ignore_ascii_case(haystack: &str, prefix: &str) -> bool {
    haystack.len() >= prefix.len()
        && haystack.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Classify severity from a raw output line using common cross-language patterns.
fn classify_severity(line: &str) -> &'static str {
    if let Some(event) = super::heuristic::try_parse_heuristic(line) {
        return match event.severity.as_deref() {
            Some("error") => "error",
            Some("warning") => "warning",
            _ => "info",
        };
    }
    "info"
}

/// Evaluate whether a stderr line looks like it contains an error, even when
/// the tool-specific parser didn't flag it. Used as a safety net for CLI tools
/// that write errors to stderr without structured formatting.
///
/// Returns true if the line strongly signals an error condition.
pub fn stderr_looks_like_error(line: &str) -> bool {
    // Strong signals: explicit error keywords
    contains_ignore_ascii_case(line, "error:")
        || contains_ignore_ascii_case(line, "error ")
        || contains_ignore_ascii_case(line, "failed:")
        || contains_ignore_ascii_case(line, "fatal:")
        || contains_ignore_ascii_case(line, "panic:")
        || contains_ignore_ascii_case(line, "panic!")
        || contains_ignore_ascii_case(line, "traceback (most recent call last)")
        || contains_ignore_ascii_case(line, "segmentation fault")
        || contains_ignore_ascii_case(line, "abort trap")
        || starts_with_ignore_ascii_case(line, "e ")
        || starts_with_ignore_ascii_case(line, "e\t")
        // Common CLI patterns
        || contains_ignore_ascii_case(line, "command not found")
        || contains_ignore_ascii_case(line, "no such file")
        || contains_ignore_ascii_case(line, "cannot find")
        || contains_ignore_ascii_case(line, "permission denied")
        || contains_ignore_ascii_case(line, "access denied")
        || contains_ignore_ascii_case(line, "not found")
        || contains_ignore_ascii_case(line, "syntax error")
        || contains_ignore_ascii_case(line, "unexpected token")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pattern(regex: &str, event_type: &str, severity: &str) -> LinePattern {
        LinePattern {
            regex: Regex::new(regex).unwrap(),
            event_type: event_type.into(),
            severity: severity.into(),
            severity_group: None,
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
    fn test_dynamic_severity_from_capture_group() {
        let parser = TomlParser::new(vec![LinePattern {
            regex: Regex::new(r"^\[(error|warning|info|weird)\]\s+(.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(), // fixed fallback
            severity_group: Some(1),
            file_group: None,
            line_group: None,
            col_group: None,
            code_group: None,
            message_group: Some(2),
            deprecated: false,
            replaced_by: None,
        }]);

        // Captured severity is normalized per line.
        let e = parser.parse_line("[error] boom").unwrap();
        assert_eq!(e.severity.as_deref(), Some("error"));
        let e = parser.parse_line("[warning] careful").unwrap();
        assert_eq!(e.severity.as_deref(), Some("warning"));
        let e = parser.parse_line("[info] note").unwrap();
        assert_eq!(e.severity.as_deref(), Some("info"));

        // Unrecognized captured text falls back to the fixed severity (fail-safe).
        let e = parser.parse_line("[weird] unknown").unwrap();
        assert_eq!(e.severity.as_deref(), Some("error"));
    }

    #[test]
    fn test_normalize_severity_edge_cases() {
        assert_eq!(normalize_severity("ERROR").as_deref(), Some("error"));
        assert_eq!(normalize_severity("Warning").as_deref(), Some("warning"));
        assert_eq!(normalize_severity("WARN").as_deref(), Some("warning"));
        assert_eq!(normalize_severity("Fatal").as_deref(), Some("error"));
        assert_eq!(normalize_severity("note:").as_deref(), Some("info"));
        assert_eq!(normalize_severity("  info ").as_deref(), Some("info"));
        assert_eq!(normalize_severity("bogus"), None);
    }

    #[test]
    fn test_basic_match() {
        let parser = TomlParser::new(vec![LinePattern {
            regex: Regex::new(r"^error: (.+)$").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            severity_group: None,
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
        let parser = TomlParser::new(vec![make_pattern(r"^error: (.+)$", "diagnostic", "error")]);
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
            severity_group: None,
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

        let success = raw_event("test result: ok. 12 passed; 0 failed", 3);
        assert_eq!(success.severity, Some("info".into()));
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
        for line in
            &["building module", "compilation successful", "12 tests passed", "installed packages"]
        {
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
