//! Format detection layer — auto-detects structured output formats.
//!
//! Runs as the first pass in the parser pipeline. When a command produces
//! structured output (JSON, NDJSON, YAML, CSV), this parser extracts
//! structured events directly, skipping the regex pipeline entirely.
//!
//! Detection order:
//! 1. Whole output is valid JSON (object or array) → parse as single event/array
//! 2. NDJSON: each line is a valid JSON object → parse each line as an event
//! 3. YAML: output starts with `---` or has key: value patterns
//! 4. CSV/TSV: consistent delimiter patterns across lines
//! 5. Neither → return None, fall through to regex pipeline

use arshy_lib::ipc::TaskEvent;

/// Try to parse accumulated output as structured data (JSON/NDJSON/YAML/CSV).
///
/// Called after command completion on the accumulated output string.
/// Returns `None` if output is not structured.
pub fn try_parse(output: &str) -> Option<Vec<TaskEvent>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Strategy 1: whole output is valid JSON
    if let Some(events) = try_parse_whole(trimmed) {
        return Some(events);
    }

    // Strategy 2: NDJSON — each line is a JSON object
    if let Some(events) = try_parse_ndjson(trimmed) {
        return Some(events);
    }

    // Strategy 3: YAML — starts with --- or has key: value patterns
    if let Some(events) = try_parse_yaml(trimmed) {
        return Some(events);
    }

    // Strategy 4: CSV/TSV — consistent delimiter patterns
    if let Some(events) = try_parse_csv(trimmed) {
        return Some(events);
    }

    None
}

/// Try parsing a single line as JSON. Used for line-by-line streaming.
/// Returns `Some(event)` if the line is valid JSON, `None` otherwise.
pub fn try_parse_line(line: &str) -> Option<TaskEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    Some(value_to_event(&value))
}

// ── Internal helpers ──────────────────────────────────────────────────────

fn try_parse_whole(output: &str) -> Option<Vec<TaskEvent>> {
    let first_char = output.chars().next()?;
    if first_char != '{' && first_char != '[' {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(output).ok()?;

    match &value {
        serde_json::Value::Array(arr) => Some(arr.iter().map(value_to_event).collect()),
        serde_json::Value::Object(_) => Some(vec![value_to_event(&value)]),
        _ => None,
    }
}

fn try_parse_ndjson(output: &str) -> Option<Vec<TaskEvent>> {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return None; // Single line is handled by try_parse_whole
    }

    // Check if at least 80% of lines are valid JSON objects
    let json_count = lines
        .iter()
        .filter(|l| l.trim().starts_with('{'))
        .filter(|l| serde_json::from_str::<serde_json::Value>(l.trim()).is_ok())
        .count();

    if json_count < (lines.len() * 4 / 5) {
        return None;
    }

    Some(lines.iter().filter_map(|l| try_parse_line(l)).collect())
}

fn value_to_event(value: &serde_json::Value) -> TaskEvent {
    match value {
        serde_json::Value::Object(obj) => {
            // Try to extract structured fields from JSON object
            let event_type = obj
                .get("type")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    obj.get("level").and_then(|v| v.as_str()).map(|l| match l {
                        "error" | "fatal" => "diagnostic",
                        "warn" | "warning" => "diagnostic",
                        _ => "log",
                    })
                })
                .unwrap_or("data")
                .to_string();

            let severity = obj
                .get("severity")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("level").and_then(|v| v.as_str()))
                .map(|s| match s {
                    "error" | "fatal" | "critical" => "error",
                    "warn" | "warning" => "warning",
                    _ => "info",
                })
                .unwrap_or("info")
                .to_string();

            let code = obj
                .get("code")
                .or_else(|| obj.get("errorCode"))
                .or_else(|| obj.get("error_code"))
                .and_then(|v| {
                    v.as_str().or_else(|| v.as_i64().map(|_| "")).map(|s| {
                        if s.is_empty() {
                            v.to_string()
                        } else {
                            s.to_string()
                        }
                    })
                })
                .or_else(|| obj.get("code").and_then(|v| v.as_i64()).map(|n| n.to_string()));

            let message = obj
                .get("message")
                .or_else(|| obj.get("msg"))
                .or_else(|| obj.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let file = obj
                .get("file")
                .or_else(|| obj.get("filename"))
                .or_else(|| obj.get("path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let line = obj
                .get("line")
                .or_else(|| obj.get("lineNumber"))
                .or_else(|| obj.get("lineno"))
                .and_then(|v| v.as_u64());

            let location = file.map(|f| arshy_lib::ipc::EventLocation {
                file: f,
                line: line.unwrap_or(0),
                column: None,
            });

            TaskEvent {
                seq: 0,
                event_type,
                severity: Some(severity),
                code,
                message: if message.is_empty() {
                    serde_json::to_string(value).unwrap_or_default()
                } else {
                    message
                },
                location,
                context: None,
                hint: None,
            }
        }
        serde_json::Value::Array(arr) => {
            // Array: summarize
            TaskEvent {
                seq: 0,
                event_type: "data".into(),
                severity: Some("info".into()),
                code: None,
                message: format!("JSON array ({} items)", arr.len()),
                location: None,
                context: None,
                hint: None,
            }
        }
        _ => TaskEvent {
            seq: 0,
            event_type: "data".into(),
            severity: Some("info".into()),
            code: None,
            message: serde_json::to_string(value).unwrap_or_default(),
            location: None,
            context: None,
            hint: None,
        },
    }
}

// ── YAML detection ──────────────────────────────────────────────────────

/// Try parsing output as YAML.
/// Detection: starts with `---` or contains key: value patterns.
fn try_parse_yaml(output: &str) -> Option<Vec<TaskEvent>> {
    let lines: Vec<&str> = output.lines().collect();
    if lines.is_empty() {
        return None;
    }

    // Check if output looks like YAML
    let is_yaml = lines[0].trim() == "---"
        || (lines.len() >= 2
            && lines.iter().take(5).all(|l| {
                let trimmed = l.trim();
                trimmed.is_empty() || trimmed.starts_with('#') || trimmed.contains(": ")
            }));

    if !is_yaml {
        return None;
    }

    // Parse YAML-like key: value pairs
    let mut events = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" || trimmed == "..." {
            continue;
        }

        if let Some((key, value)) = trimmed.split_once(": ") {
            events.push(TaskEvent {
                seq: 0,
                event_type: "data".into(),
                severity: Some("info".into()),
                code: Some(key.to_string()),
                message: value.to_string(),
                location: None,
                context: None,
                hint: None,
            });
        }
    }

    if events.is_empty() {
        None
    } else {
        Some(events)
    }
}

// ── CSV/TSV detection ──────────────────────────────────────────────────

/// Try parsing output as CSV/TSV.
/// Detection: consistent delimiter patterns across lines.
fn try_parse_csv(output: &str) -> Option<Vec<TaskEvent>> {
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return None;
    }

    // Detect delimiter: tab, comma, or pipe
    let delimiter = if lines[0].contains('\t') {
        '\t'
    } else if lines[0].contains(',') {
        ','
    } else if lines[0].contains('|') {
        '|'
    } else {
        return None;
    };

    // Check if all lines have consistent column count
    let col_count = lines[0].split(delimiter).count();
    if col_count < 2 {
        return None;
    }

    let consistent = lines.iter().all(|l| l.split(delimiter).count() == col_count);
    if !consistent {
        return None;
    }

    // Parse as CSV/TSV
    let mut events = Vec::new();
    for line in lines {
        let cols: Vec<&str> = line.split(delimiter).collect();
        let message = cols.join(", ");
        events.push(TaskEvent {
            seq: 0,
            event_type: "data".into(),
            severity: Some("info".into()),
            code: None,
            message,
            location: None,
            context: None,
            hint: None,
        });
    }

    if events.is_empty() {
        None
    } else {
        Some(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Whole JSON ───────────────────────────────────────────────────────

    #[test]
    fn empty_returns_none() {
        assert!(try_parse("").is_none());
        assert!(try_parse("   ").is_none());
    }

    #[test]
    fn non_json_returns_none() {
        assert!(try_parse("hello world").is_none());
        assert!(try_parse("error: something").is_none());
    }

    #[test]
    fn json_object_single_event() {
        let output = r#"{"status":"ok","count":42}"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn json_array_multiple_events() {
        let output = r#"[{"name":"a"},{"name":"b"}]"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn json_object_with_error_fields() {
        let output = r#"{"level":"error","code":"E001","message":"disk full","file":"/dev/sda"}"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "diagnostic");
        assert_eq!(events[0].severity.as_deref(), Some("error"));
        assert_eq!(events[0].code.as_deref(), Some("E001"));
        assert_eq!(events[0].message, "disk full");
        assert_eq!(events[0].location.as_ref().unwrap().file, "/dev/sda");
    }

    #[test]
    fn json_with_level_field() {
        let output = r#"{"level":"warn","msg":"deprecated API"}"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events[0].event_type, "diagnostic");
        assert_eq!(events[0].severity.as_deref(), Some("warning"));
    }

    // ── NDJSON ──────────────────────────────────────────────────────────

    #[test]
    fn ndjson_multiple_lines() {
        let output = r#"{"ts":"2024-01-01","level":"info","msg":"started"}
{"ts":"2024-01-01","level":"error","msg":"failed"}
{"ts":"2024-01-01","level":"info","msg":"retried"}"#;
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[1].severity.as_deref(), Some("error"));
    }

    #[test]
    fn ndjson_rejects_mixed_output() {
        let output = r#"{"level":"info","msg":"started"}
some random text line
{"level":"error","msg":"failed"}
more random text"#;
        // Only 50% are JSON, below 80% threshold
        assert!(try_parse(output).is_none());
    }

    #[test]
    fn ndjson_accepts_mostly_json() {
        let output = r#"{"level":"info","msg":"started"}
{"level":"info","msg":"processing"}
{"level":"info","msg":"done"}
some log line"#;
        // 75% are JSON (3/4) — meets the 75% threshold (lines * 4 / 5)
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3, "should parse 3 JSON lines");
    }

    #[test]
    fn ndjson_rejects_low_json_ratio() {
        let output = r#"{"level":"info","msg":"started"}
some random text
another text line
{"level":"info","msg":"done"}"#;
        // 50% are JSON (2/4) — below 75% threshold
        assert!(try_parse(output).is_none());
    }

    // ── Single line parsing ──────────────────────────────────────────────

    #[test]
    fn parse_line_valid_json() {
        let event = try_parse_line(r#"{"level":"error","message":"oops"}"#).unwrap();
        assert_eq!(event.severity.as_deref(), Some("error"));
    }

    #[test]
    fn parse_line_non_json_returns_none() {
        assert!(try_parse_line("hello world").is_none());
        assert!(try_parse_line("").is_none());
    }

    // ── Edge cases ──────────────────────────────────────────────────────

    #[test]
    fn empty_json_array() {
        let events = try_parse("[]").unwrap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn empty_json_object() {
        let events = try_parse("{}").unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn json_with_leading_whitespace() {
        let output = "\n  {\"key\":\"value\"}";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 1);
    }

    // ── YAML detection ──────────────────────────────────────────────────

    #[test]
    fn yaml_document_separator() {
        let output = "---\nname: test\nstatus: ok\ncount: 42";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].code.as_deref(), Some("name"));
        assert_eq!(events[0].message, "test");
    }

    #[test]
    fn yaml_key_value_pairs() {
        let output = "name: test\nstatus: ok\ncount: 42";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn yaml_with_comments() {
        let output = "# Config file\nname: test\n# Status\nstatus: ok";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 2);
    }

    // ── CSV/TSV detection ──────────────────────────────────────────────

    #[test]
    fn tsv_detection() {
        let output = "name\tage\tcity\nAlice\t30\tNYC\nBob\t25\tLA";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3);
        assert!(events[1].message.contains("Alice"));
    }

    #[test]
    fn csv_detection() {
        let output = "name,age,city\nAlice,30,NYC\nBob,25,LA";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn pipe_delimited() {
        let output = "name|status|count\nok|pass|42\nerror|fail|1";
        let events = try_parse(output).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn inconsistent_columns_rejected() {
        let output = "a,b,c\n1,2\n3,4,5";
        // Inconsistent column count — should not be parsed as CSV
        assert!(try_parse(output).is_none());
    }
}
