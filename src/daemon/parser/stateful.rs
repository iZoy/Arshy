//! Stateful pattern-based parser for cross-line parsing.
//!
//! Regex + state-machine patterns loaded from TOML parser definitions.
//! Tracks state across lines for tools whose output spans multiple lines
//! (e.g., npm install summary, webpack error blocks).

use crate::ipc::{EventLocation, TaskEvent};
use regex::Regex;

/// A stateful parser for cross-line pattern matching.
///
/// Patterns are regex rules with optional state conditions and transitions,
/// loaded from TOML parser definitions via the registry.
pub struct StatefulParser {
    name: String,
    patterns: Vec<StatefulPattern>,
    state: std::sync::Mutex<ParserState>,
}

/// A regex pattern with state-machine transitions.
///
/// Used for tools whose output spans multiple lines and requires
/// tracking state across lines (e.g., npm error blocks, webpack chunks).
#[derive(Debug, Clone)]
pub struct StatefulPattern {
    pub regex: Regex,
    pub event_type: String,
    pub severity: String,
    pub severity_group: Option<usize>,
    pub message_group: Option<usize>,
    pub file_group: Option<usize>,
    pub line_group: Option<usize>,
    /// If set, this pattern only matches when state key has this value.
    pub state_condition: Option<(String, String)>,
    /// If set, transitions to this state after matching.
    pub state_transition: Option<(String, String)>,
    /// When true, this pattern is deprecated.
    pub deprecated: bool,
    /// Name of the replacement pattern, if any.
    pub replaced_by: Option<String>,
}

#[derive(Default)]
struct ParserState {
    values: std::collections::HashMap<String, String>,
}

impl StatefulParser {
    /// Create with pre-loaded patterns (from TOML definitions via registry).
    pub fn with_patterns(name: &str, patterns: Vec<StatefulPattern>) -> Self {
        Self {
            name: name.to_string(),
            patterns,
            state: std::sync::Mutex::new(ParserState::default()),
        }
    }

    /// Legacy constructor using builtin patterns. Prefer `with_patterns`.
    #[allow(dead_code)]
    pub fn new(name: &str) -> Self {
        Self::with_patterns(name, builtin_stateful_patterns(name))
    }

    /// Parser name.
    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Feed one line to the stateful parser. Returns any events generated.
    pub fn feed_line(&self, line: &str, seq: u64) -> Vec<TaskEvent> {
        feed_patterns(&self.patterns, &self.state, line, seq)
    }

    /// Called on command completion — emit final events.
    pub fn on_complete(&self, exit_code: i32, seq: u64) -> Vec<TaskEvent> {
        on_complete_patterns(&self.name, &self.state, exit_code, seq)
    }

    /// Reset parser state (for reuse).
    #[allow(dead_code)]
    pub fn reset(&self) {
        *self.state.lock().unwrap() = ParserState::default();
    }
}

fn feed_patterns(
    patterns: &[StatefulPattern],
    state: &std::sync::Mutex<ParserState>,
    line: &str,
    seq: u64,
) -> Vec<TaskEvent> {
    let mut events = Vec::new();
    let mut state = state.lock().unwrap();

    for pat in patterns {
        if let Some((key, value)) = &pat.state_condition {
            match state.values.get(key.as_str()) {
                Some(v) if v == value => {}
                _ => continue,
            }
        }

        if let Some(caps) = pat.regex.captures(line) {
            let message = pat
                .message_group
                .and_then(|i| caps.get(i))
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_else(|| line.to_string());

            let location = if let Some(fi) = pat.file_group {
                caps.get(fi).map(|m| EventLocation {
                    file: m.as_str().to_string(),
                    line: pat
                        .line_group
                        .and_then(|li| caps.get(li))
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0),
                    column: None,
                })
            } else {
                None
            };

            let severity = pat
                .severity_group
                .and_then(|g| caps.get(g))
                .and_then(|m| super::toml::normalize_severity(m.as_str()))
                .unwrap_or_else(|| pat.severity.clone());

            events.push(TaskEvent {
                seq: seq + events.len() as u64,
                event_type: pat.event_type.clone(),
                severity: Some(severity),
                code: None,
                message,
                location,
                context: None,
                hint: None,
            });

            if let Some((key, value)) = &pat.state_transition {
                state.values.insert(key.clone(), value.clone());
            }
        }
    }

    events
}

fn on_complete_patterns(
    name: &str,
    state: &std::sync::Mutex<ParserState>,
    exit_code: i32,
    seq: u64,
) -> Vec<TaskEvent> {
    let mut events = Vec::new();

    if name == "npm" {
        let state = state.lock().unwrap();
        let has_adds = state.values.contains_key("packages_added");
        let has_error = state.values.get("has_error").is_some_and(|v| v == "true");

        if has_adds || has_error {
            let severity = if has_error || exit_code != 0 { "error" } else { "info" };
            let msg = if has_adds {
                format!("npm install completed (exit {})", exit_code)
            } else {
                format!("npm install failed (exit {})", exit_code)
            };
            events.push(TaskEvent {
                seq,
                event_type: "summary".into(),
                severity: Some(severity.into()),
                code: None,
                message: msg,
                location: None,
                context: None,
                hint: None,
            });
        }
    }

    events
}

/// Built-in stateful patterns for cross-line tools.
fn builtin_stateful_patterns(tool: &str) -> Vec<StatefulPattern> {
    match tool {
        "npm" => npm_stateful_patterns(),
        "webpack" => webpack_patterns(),
        _ => vec![],
    }
}

fn npm_stateful_patterns() -> Vec<StatefulPattern> {
    vec![
        // added 42 packages in 3s
        StatefulPattern {
            regex: Regex::new(r"^added (\d+) packages?").unwrap(),
            event_type: "summary".into(),
            severity: "info".into(),
            severity_group: None,
            message_group: None,
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: Some(("packages_added".into(), "done".into())),
            deprecated: false,
            replaced_by: None,
        },
        // npm ERR! code ERESOLVE
        StatefulPattern {
            regex: Regex::new(r"^npm ERR! (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            severity_group: None,
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: Some(("has_error".into(), "true".into())),
            deprecated: false,
            replaced_by: None,
        },
        // npm WARN deprecated ...
        StatefulPattern {
            regex: Regex::new(r"^npm WARN (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "warning".into(),
            severity_group: None,
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
            deprecated: false,
            replaced_by: None,
        },
        // up to date, audited X packages
        StatefulPattern {
            regex: Regex::new(r"^up to date,? audited (\d+) packages?").unwrap(),
            event_type: "summary".into(),
            severity: "info".into(),
            severity_group: None,
            message_group: None,
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
            deprecated: false,
            replaced_by: None,
        },
        // audited X packages in Ys
        StatefulPattern {
            regex: Regex::new(r"^audited (\d+) packages?").unwrap(),
            event_type: "summary".into(),
            severity: "info".into(),
            severity_group: None,
            message_group: None,
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
            deprecated: false,
            replaced_by: None,
        },
    ]
}

fn webpack_patterns() -> Vec<StatefulPattern> {
    vec![
        // ERROR in ./src/index.ts
        StatefulPattern {
            regex: Regex::new(r"^ERROR in (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "error".into(),
            severity_group: None,
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: Some(("has_error".into(), "true".into())),
            deprecated: false,
            replaced_by: None,
        },
        // WARNING in ./src/index.ts
        StatefulPattern {
            regex: Regex::new(r"^WARNING in (.+)").unwrap(),
            event_type: "diagnostic".into(),
            severity: "warning".into(),
            severity_group: None,
            message_group: Some(1),
            file_group: None,
            line_group: None,
            state_condition: None,
            state_transition: None,
            deprecated: false,
            replaced_by: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_npm_error() {
        let parser = StatefulParser::new("npm");
        let events = parser.feed_line("npm ERR! code ERESOLVE", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].severity, Some("error".into()));
        assert!(events[0].message.contains("ERESOLVE"));
    }

    #[test]
    fn test_npm_warn() {
        let parser = StatefulParser::new("npm");
        let events = parser.feed_line("npm WARN deprecated request@2.88.2", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].severity, Some("warning".into()));
    }

    #[test]
    fn test_npm_summary() {
        let parser = StatefulParser::new("npm");
        let events = parser.feed_line("added 42 packages in 3s", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "summary");
    }

    #[test]
    fn test_npm_on_complete_with_error() {
        let parser = StatefulParser::new("npm");
        parser.feed_line("npm ERR! code ENOENT", 1);
        let events = parser.on_complete(1, 2);
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("failed"));
        assert_eq!(events[0].severity, Some("error".into()));
    }

    #[test]
    fn test_npm_on_complete_success() {
        let parser = StatefulParser::new("npm");
        parser.feed_line("added 42 packages in 3s", 1);
        let events = parser.on_complete(0, 2);
        assert_eq!(events.len(), 1);
        assert!(events[0].message.contains("completed"));
        assert_eq!(events[0].severity, Some("info".into()));
    }

    #[test]
    fn test_webpack_error() {
        let parser = StatefulParser::new("webpack");
        let events = parser.feed_line("ERROR in ./src/index.ts", 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].severity, Some("error".into()));
    }

    #[test]
    fn test_unknown_tool_no_patterns() {
        let parser = StatefulParser::new("unknown_tool");
        let events = parser.feed_line("some output line", 1);
        assert!(events.is_empty());
    }
}
