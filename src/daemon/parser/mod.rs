//! Parser engine — TOML (Tier 1) + Stateful (Tier 2) + Crash + Heuristic + Raw fallback.
//!
//! Coverage target: 70% TOML regex, 25% stateful, 5% crash/raw.
//!
//! Architecture:
//! - `toml_def` — TOML file format deserialization
//! - `toml` — stateless line-by-line matching
//! - `rhai` — stateful cross-line matching
//! - `crash` — universal crash/traceback detection
//! - `registry` — loads and deduplicates parser entries
//! - `detect` — tool extraction and version detection
//! - `loader` — filesystem hot-reload watcher (notify v7)

mod crash;
pub(crate) mod dedup;
mod detect;
mod heuristic;
pub mod hint;
mod json;
mod loader;
pub mod pair_merger;
mod redos;
mod registry;
pub mod rhai;
pub mod toml;
pub mod toml_def;

pub use json::try_parse as try_parse_json;
pub use toml::stderr_looks_like_error;

pub use detect::*;
pub use registry::*;

use crate::config::ParserConfig;
use crate::ipc::{EventContext, TaskEvent};
use crate::Result;
use std::sync::{Arc, RwLock};

/// Central parser engine — detects tools, loads parsers, dispatches lines.
///
/// The registry is wrapped in `RwLock` to support hot-reload from the
/// filesystem watcher thread. Reads (parsing) are lock-free in practice;
/// writes (reload) happen only when parser files change.
pub struct Engine {
    registry: Arc<RwLock<ParserRegistry>>,
    config: ParserConfig,
}

impl Engine {
    /// Build the engine from config: load builtin + filesystem parsers.
    pub fn new(config: &ParserConfig) -> Result<Self> {
        let registry = ParserRegistry::load(config)?;
        Ok(Self { registry: Arc::new(RwLock::new(registry)), config: config.clone() })
    }

    /// Start the filesystem watcher for hot-reload (if configured).
    /// Returns the watcher handle; dropping it stops watching.
    pub fn start_watcher(&self) -> Result<Option<loader::ParserWatcher>> {
        if !self.config.hot_reload {
            return Ok(None);
        }
        let registry = self.registry.clone();
        let config = self.config.clone();
        let watcher = loader::ParserWatcher::start(&self.config.dirs, move || {
            match ParserRegistry::load(&config) {
                Ok(new_registry) => match registry.write() {
                    Ok(mut reg) => {
                        let audit = reg.diff(&new_registry);
                        tracing::info!("parser hot-reload: {}", audit);
                        *reg = new_registry;
                    }
                    Err(e) => tracing::error!("registry lock poisoned: {}", e),
                },
                Err(e) => tracing::error!("parser reload failed: {}", e),
            }
        })?;
        Ok(Some(watcher))
    }

    /// Reload parsers from disk (manual trigger).
    /// Returns a human-readable diff of what changed.
    #[allow(dead_code)]
    pub fn reload(&self) -> Result<String> {
        let new_registry = ParserRegistry::load(&self.config)?;
        let mut reg = self
            .registry
            .write()
            .map_err(|_| crate::ArshyError::Other("registry lock poisoned".into()))?;
        let audit = reg.diff(&new_registry);
        tracing::info!("parser reload: {}", audit);
        *reg = new_registry;
        Ok(audit)
    }

    /// Detect the tool from a command string.
    pub fn detect(&self, command: &str) -> Option<ParsedTool> {
        self.registry.read().ok()?.detect(command)
    }

    /// Look up a parser by name (for testing/harness use).
    #[allow(dead_code)]
    pub fn get_by_name(&self, name: &str) -> Option<ParsedTool> {
        let reg = self.registry.read().ok()?;
        let entry = reg.get(name)?;
        Some(ParsedTool {
            tool_name: entry.tool_name.clone(),
            parser_name: entry.name.clone(),
            parser_type: entry.parser_type.clone(),
            version: None,
        })
    }

    /// Check whether a parser's version constraints are satisfied by a probed version.
    /// Returns true if the parser has no constraints or the version satisfies them.
    pub fn is_version_compatible(&self, parser_name: &str, version: &str) -> bool {
        let reg = match self.registry.read() {
            Ok(r) => r,
            Err(_) => return true, // can't read registry — don't filter
        };
        match reg.get(parser_name) {
            Some(entry) => detect::version_satisfies(
                version,
                entry.min_version.as_deref(),
                entry.max_version.as_deref(),
            ),
            None => true,
        }
    }

    /// Create a parser session for a task.
    pub fn create_session(&self, tool: Option<&ParsedTool>) -> ParserSession {
        let reg = match self.registry.read() {
            Ok(r) => r,
            Err(_) => return ParserSession::raw(),
        };

        match tool {
            Some(t) => {
                if let Some(entry) = reg.get(&t.parser_name) {
                    // Audit: warn on deprecated patterns
                    if entry.deprecated_count > 0 {
                        tracing::warn!(
                            "parser '{}' has {} deprecated pattern(s); consider updating",
                            entry.name,
                            entry.deprecated_count
                        );
                    }

                    // Filter line patterns: skip deprecated patterns that have a replacement
                    let line_patterns: Vec<toml::LinePattern> = entry
                        .line_patterns
                        .iter()
                        .filter(|p| {
                            if p.deprecated {
                                if p.replaced_by.is_some() {
                                    return false; // replacement exists, skip deprecated
                                }
                                tracing::warn!(
                                    "parser '{}': pattern '{}' is deprecated with no replacement",
                                    entry.name,
                                    p.regex.as_str()
                                );
                            }
                            true
                        })
                        .cloned()
                        .collect();

                    // Filter stateful patterns similarly
                    let stateful_patterns: Vec<rhai::StatefulPattern> = entry.stateful_patterns.iter()
                        .filter(|p| {
                            if p.deprecated {
                                if p.replaced_by.is_some() {
                                    return false;
                                }
                                tracing::warn!(
                                    "parser '{}': stateful pattern is deprecated with no replacement",
                                    entry.name
                                );
                            }
                            true
                        })
                        .cloned()
                        .collect();

                    let stateful = if !stateful_patterns.is_empty() {
                        Some(rhai::StatefulParser::with_patterns(&entry.name, stateful_patterns))
                    } else if let Some(ref script) = entry.rhai_script {
                        match rhai::StatefulParser::with_script(script) {
                            Ok(p) => Some(p),
                            Err(e) => {
                                tracing::error!("rhai script '{}': {}", entry.name, e);
                                None
                            }
                        }
                    } else {
                        None
                    };

                    return ParserSession {
                        toml_parser: if line_patterns.is_empty() {
                            None
                        } else {
                            Some(toml::TomlParser::new(line_patterns))
                        },
                        stateful,
                    };
                }
                ParserSession::raw()
            }
            None => ParserSession::raw(),
        }
    }

    /// Parse a single output line (stateless, for non-session use).
    #[allow(dead_code)]
    pub fn parse_line(&self, line: &str, seq: u64, tool: Option<&ParsedTool>) -> TaskEvent {
        let reg = match self.registry.read() {
            Ok(r) => r,
            Err(_) => return toml::raw_event(line, seq),
        };

        if let Some(t) = tool {
            if let Some(entry) = reg.get(&t.parser_name) {
                if !entry.line_patterns.is_empty() {
                    let parser = toml::TomlParser::new(entry.line_patterns.clone());
                    if let Some(mut event) = parser.parse_line(line) {
                        event.seq = seq;
                        return event;
                    }
                }
            }
        }
        toml::raw_event(line, seq)
    }

    pub fn config(&self) -> &ParserConfig {
        &self.config
    }
}

/// A per-task parser session that holds a pre-built TOML parser and state.
pub struct ParserSession {
    toml_parser: Option<toml::TomlParser>,
    stateful: Option<rhai::StatefulParser>,
}

impl ParserSession {
    fn raw() -> Self {
        Self { toml_parser: None, stateful: None }
    }

    /// Parse one line of output.
    ///
    /// Pipeline order:
    /// 1. Format detection (JSON line) — highest priority for structured data
    /// 2. Stateful parser (Rhai/state-machine)
    /// 3. TOML parser (regex patterns)
    /// 4. Crash parser (universal crash detection)
    /// 5. Heuristic error filter
    /// 6. Raw fallback
    ///
    /// Returns zero or more events. Context lines from rustc-style diagnostics
    /// (pipe/caret/note markers) are merged into the preceding diagnostic event
    /// rather than emitted as separate log events.
    pub fn parse_line(&self, line: &str, seq: u64, _tool: Option<&ParsedTool>) -> Vec<TaskEvent> {
        // 1. Format detection — try JSON first for structured output
        if let Some(mut event) = json::try_parse_line(line) {
            event.seq = seq;
            return vec![event];
        }

        // 2. Stateful parser (if it matched, return; otherwise fall through)
        if let Some(ref stateful) = self.stateful {
            let events = stateful.feed_line(line, seq);
            if !events.is_empty() {
                return events;
            }
            // Stateful parser didn't match — fall through to crash/raw
        }

        // 3. TOML parser (pre-built at session creation, no per-line clone)
        if let Some(ref parser) = self.toml_parser {
            if let Some(mut event) = parser.parse_line(line) {
                event.seq = seq;
                return vec![event];
            }
        }

        // 4. Crash parser (universal, no tool dependency)
        if let Some(mut event) = crash::try_parse_crash(line) {
            event.seq = seq;
            return vec![event];
        }

        // 4.5 Heuristic error filter (keyword-based, no tool dependency)
        if let Some(mut event) = heuristic::try_parse_heuristic(line) {
            event.seq = seq;
            return vec![event];
        }

        // 5. Raw fallback
        vec![toml::raw_event(line, seq)]
    }

    /// Called on command completion — emit final events from stateful parsers.
    pub fn on_complete(&self, exit_code: i32, seq: u64) -> Vec<TaskEvent> {
        if let Some(ref stateful) = self.stateful {
            return stateful.on_complete(exit_code, seq);
        }
        vec![]
    }
}

/// Metadata about a matched parser for a given command.
#[derive(Debug, Clone)]
pub struct ParsedTool {
    pub tool_name: String,
    pub parser_name: String,
    #[allow(dead_code)]
    pub parser_type: ParserType,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParserType {
    Toml,
    Rhai,
    Raw,
}

/// Post-processor that merges rustc-style diagnostic context lines into the
/// preceding diagnostic event's `context` fields, instead of emitting them as
/// separate log events.
///
/// Context lines match these patterns:
/// - `  |` or `  -->` (pipe/arrow markers)
/// - `N | <code>` (numbered source lines)
/// - `  |   ^^^^ expected ...` (caret markers)
/// - `  |   expected due to this` (explanation)
/// - `  = note: ...` or `  = help: ...` (note/help directives)
///
/// The merged lines are appended to the parent event's `context.after` list,
/// and the separate context events are suppressed.
pub struct RustcContextMerger {
    /// Buffer for the last diagnostic/log event that may receive merged context.
    pending: Option<TaskEvent>,
    /// Collected context lines to attach to `pending`.
    context_lines: Vec<String>,
    /// Counter of merged context events (for observability).
    merged_count: u64,
}

impl Default for RustcContextMerger {
    fn default() -> Self {
        Self::new()
    }
}

impl RustcContextMerger {
    pub fn new() -> Self {
        Self { pending: None, context_lines: Vec::new(), merged_count: 0 }
    }

    /// Total context lines merged so far.
    #[allow(dead_code)]
    pub fn merged_count(&self) -> u64 {
        self.merged_count
    }

    /// Feed an event through the merger. Returns `Some(event)` when an event
    /// is ready to be emitted; returns `None` if the event was absorbed as context.
    pub fn feed(&mut self, event: TaskEvent) -> Option<TaskEvent> {
        if is_rustc_context_line(&event.message) {
            // This is a context line — buffer it and suppress the event.
            self.context_lines.push(event.message.clone());
            self.merged_count += 1;
            return None;
        }

        // Not a context line — flush whatever we have buffered.
        let flushed = self.flush_pending();

        // If the incoming event is a diagnostic or log that could receive context,
        // buffer it; otherwise emit it directly.
        if event.event_type == "diagnostic" || event.event_type == "log" {
            self.pending = Some(event);
        } else {
            // Non-diagnostic events (test_result, summary, etc.) pass through immediately.
            return flushed.or(Some(event));
        }

        flushed
    }

    /// Flush any remaining buffered event (call at end of stream).
    pub fn finish(&mut self) -> Option<TaskEvent> {
        self.flush_pending()
    }

    fn flush_pending(&mut self) -> Option<TaskEvent> {
        let mut event = self.pending.take()?;
        if !self.context_lines.is_empty() {
            // Filter out noise lines: empty pipes, pure caret/arrow markers
            let merged: Vec<String> = self
                .context_lines
                .drain(..)
                .filter(|line| {
                    let trimmed = line.trim();
                    // Remove empty lines
                    if trimmed.is_empty() {
                        return false;
                    }
                    // Remove pure caret/dash/pipe connector lines: "  |  ^^^^", "  |  ---"
                    // Keep lines with actual text (explanations, source code)
                    if let Some(after_pipe) = trimmed.strip_prefix('|') {
                        let after_pipe = after_pipe.trim();
                        // Pure markers: "|", "|  |", "|  ^^^^", "|  ---"
                        if after_pipe.is_empty()
                            || after_pipe.chars().all(|c| c == '^' || c == '-' || c == '|')
                        {
                            return false;
                        }
                    }
                    // Remove arrow markers: "--> file:line:col"
                    if trimmed.starts_with("-->") {
                        return false;
                    }
                    // Remove standalone close braces (formatting noise)
                    if trimmed == "}" {
                        return false;
                    }
                    // Remove ANSI escape lines (timestamps, log levels)
                    if trimmed.contains('\x1b') {
                        return false;
                    }
                    // Remove standalone doc comment markers
                    if trimmed == "///" || trimmed == "//" {
                        return false;
                    }
                    true
                })
                .collect();
            if !merged.is_empty() {
                match event.context {
                    Some(ref mut ctx) => {
                        ctx.after.extend(merged);
                    }
                    None => {
                        event.context = Some(EventContext {
                            before: Vec::new(),
                            line: event.message.clone(),
                            after: merged,
                        });
                    }
                }
            }
        }
        Some(event)
    }
}

/// Returns `true` if a message looks like a rustc diagnostic context line that
/// should be merged into the preceding diagnostic event rather than stored
/// separately.
fn is_rustc_context_line(msg: &str) -> bool {
    let trimmed = msg.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Pattern 1: `= note: ...`, `= help: ...`, `= warning: ...`  (rustc directives)
    if let Some(rest) = trimmed.strip_prefix("= ") {
        // Must look like a directive: `= <word>: ...`
        if let Some(colon_pos) = rest.find(':') {
            let directive = &rest[..colon_pos];
            if directive.chars().all(|c| c.is_ascii_alphabetic()) {
                return true;
            }
        }
    }

    // Pattern 2: Bare pipe markers — `|`, `| expected ...`, `|     ^^^^ ...`
    // These start with optional whitespace, then `|`.
    if trimmed.starts_with('|') {
        // `|` alone, or `| ^^^^ ...`, or `| expected ...`, etc.
        // All are valid context lines.
        return true;
    }

    // Pattern 3: Numbered source lines — `<digits> | <code>`
    // e.g. `  2 |     let x: i32 = "hello";`
    if let Some(pos) = trimmed.find(" | ") {
        let num_part = &trimmed[..pos];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit() || c == ' ') {
            return true;
        }
    }

    // Pattern 4: Arrow markers — `  -->`, `  ^^^`, `  ---`
    // Arrow: starts with `-->` after optional whitespace
    if trimmed.starts_with("-->") {
        return true;
    }

    false
}

// ── RustcContextMerger tests ──────────────────────────────────────────────────

#[cfg(test)]
mod context_merger_tests {
    use super::*;
    use crate::ipc::EventContext;

    fn make_log(message: &str, seq: u64) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "log".into(),
            severity: Some("info".into()),
            code: None,
            message: message.into(),
            location: None,
            context: None,
            hint: None,
        }
    }

    fn make_diag(message: &str, seq: u64) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: message.into(),
            location: Some(crate::ipc::EventLocation {
                file: "src/main.rs".into(),
                line: 10,
                column: None,
            }),
            context: None,
            hint: None,
        }
    }

    fn make_summary(message: &str, seq: u64) -> TaskEvent {
        TaskEvent {
            seq,
            event_type: "summary".into(),
            severity: Some("info".into()),
            code: None,
            message: message.into(),
            location: None,
            context: None,
            hint: None,
        }
    }

    #[test]
    fn context_line_detection() {
        // Pipe markers
        assert!(is_rustc_context_line("|"));
        assert!(is_rustc_context_line("  | expected `i32`, found `&str`"));
        assert!(is_rustc_context_line("  |     ^^^^ expected `i32`"));
        // Numbered source lines
        assert!(is_rustc_context_line("  2 |     let x: i32 = \"hello\";"));
        assert!(is_rustc_context_line("2 | let x = 1;"));
        // Arrow markers
        assert!(is_rustc_context_line("  --> src/main.rs:5:10"));
        assert!(is_rustc_context_line("--> file.rs:1:1"));
        // Directive notes
        assert!(is_rustc_context_line("  = note: expected due to this"));
        assert!(is_rustc_context_line("  = help: consider using `to_string()`"));
        // Non-context lines
        assert!(!is_rustc_context_line("error[E0308]: mismatched types"));
        assert!(!is_rustc_context_line("  = some_colon: not a directive"));
        assert!(!is_rustc_context_line(""));
        assert!(!is_rustc_context_line("   "));
    }

    #[test]
    fn merger_passthrough_non_context() {
        let mut m = RustcContextMerger::new();
        // First event is always buffered (merger needs to see next event)
        let e = make_diag("error[E0308]: mismatched types", 0);
        let result = m.feed(e);
        assert!(result.is_none(), "first event is buffered");
        // Second non-context event flushes the first
        let result = m.feed(make_diag("warning: unused", 1));
        assert!(result.is_some());
        assert_eq!(result.unwrap().message, "error[E0308]: mismatched types");
        // finish() flushes the second
        let final_event = m.finish();
        assert!(final_event.is_some());
        assert_eq!(final_event.unwrap().message, "warning: unused");
    }

    #[test]
    fn merger_absorbs_context_into_diagnostic() {
        let mut m = RustcContextMerger::new();
        // Diagnostic event — buffered as pending
        let diag = make_diag("error[E0308]: mismatched types", 0);
        assert!(m.feed(diag).is_none());
        // Context lines get absorbed (return None, don't flush pending)
        assert!(m.feed(make_log("  2 |     let x: i32 = \"hello\";", 1)).is_none());
        assert!(m
            .feed(make_log("  |                       ^^^^^^^ expected `i32`, found `&str`", 2))
            .is_none());
        assert!(m.feed(make_log("  |", 3)).is_none());
        assert!(m.feed(make_log("  | expected due to this", 4)).is_none());
        // Non-context line flushes the buffered event (with merged context)
        let next = make_diag("warning: unused variable", 5);
        let flushed = m.feed(next);
        assert!(flushed.is_some());
        let flushed = flushed.unwrap();
        assert_eq!(flushed.message, "error[E0308]: mismatched types");
        let ctx = flushed.context.expect("should have context");
        // Empty "|" line is filtered out, so 3 lines instead of 4
        assert_eq!(ctx.after.len(), 3);
        assert_eq!(ctx.after[0], "  2 |     let x: i32 = \"hello\";");
        assert_eq!(ctx.after[2], "  | expected due to this");
        // The second diagnostic is now pending
        let final_event = m.finish();
        assert!(final_event.is_some());
        assert_eq!(final_event.unwrap().message, "warning: unused variable");
    }

    #[test]
    fn merger_extends_existing_context() {
        let mut m = RustcContextMerger::new();
        let diag = TaskEvent {
            seq: 0,
            event_type: "diagnostic".into(),
            severity: Some("error".into()),
            code: None,
            message: "mismatched types".into(),
            location: Some(crate::ipc::EventLocation {
                file: "src/main.rs".into(),
                line: 42,
                column: None,
            }),
            context: Some(EventContext {
                before: vec!["fn main() {".into()],
                line: "    let x: i32 = \"hello\";".into(),
                after: vec!["}".into()],
            }),
            hint: None,
        };
        // First event buffered
        assert!(m.feed(diag).is_none());
        // Context lines
        assert!(m.feed(make_log("  = note: expected `i32`", 1)).is_none());
        assert!(m.feed(make_log("  = note: found `&str`", 2)).is_none());
        // Next non-context event flushes
        let next = make_log("other line", 3);
        let flushed = m.feed(next);
        assert!(flushed.is_some());
        let ctx = flushed.unwrap().context.unwrap();
        // Original after + merged context lines
        assert_eq!(
            ctx.after,
            vec![
                "}".to_string(),
                "  = note: expected `i32`".to_string(),
                "  = note: found `&str`".to_string()
            ]
        );
    }

    #[test]
    fn merger_passes_through_non_diagnostic_events() {
        let mut m = RustcContextMerger::new();
        // Summary events should pass through immediately
        let result = m.feed(make_summary("test result: ok", 0));
        assert!(result.is_some());
        assert_eq!(result.unwrap().event_type, "summary");
    }

    #[test]
    fn merger_finish_flushes_pending() {
        let mut m = RustcContextMerger::new();
        m.feed(make_diag("error: something", 0));
        m.feed(make_log("  | context line", 1));
        let result = m.finish();
        assert!(result.is_some());
        let event = result.unwrap();
        assert_eq!(event.message, "error: something");
        let ctx = event.context.unwrap();
        assert_eq!(ctx.after, vec!["  | context line"]);
    }

    #[test]
    fn merger_count_tracks_merged() {
        let mut m = RustcContextMerger::new();
        m.feed(make_diag("error: something", 0));
        assert_eq!(m.merged_count(), 0);
        m.feed(make_log("  | line 1", 1));
        assert_eq!(m.merged_count(), 1);
        m.feed(make_log("  | line 2", 2));
        assert_eq!(m.merged_count(), 2);
        m.finish();
        assert_eq!(m.merged_count(), 2);
    }
}

// ── Parser harness tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod harness_tests {
    use super::*;

    /// Per-field match statistics for parser quality measurement.
    struct FieldStats {
        total: usize,
        type_ok: usize,
        severity_ok: usize,
        code_ok: usize,
        file_ok: usize,
        line_ok: usize,
    }

    fn run_fixture(
        parser_name: &str,
        txt_path: &std::path::Path,
        json_path: &std::path::Path,
    ) -> (usize, usize, FieldStats) {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();
        // Try direct name lookup first (parser name != command name for multi-word tools)
        let tool = engine.get_by_name(parser_name).or_else(|| engine.detect(parser_name));
        let session = engine.create_session(tool.as_ref());

        let txt = std::fs::read_to_string(txt_path).unwrap();

        let lines: Vec<&str> = txt.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut all_events: Vec<TaskEvent> = Vec::new();
        for line in &lines {
            all_events.extend(session.parse_line(line, 0, tool.as_ref()));
        }

        // Apply pair merger so fixture expectations match exec-pipeline output.
        let (merged_events, _pairs_merged) =
            pair_merger::merge_diagnostic_location_pairs(all_events);
        all_events = merged_events;

        // ── Bless mode: write actual output as expected JSON ──────────────
        if std::env::var("ARSHY_BLESS").is_ok() {
            let events_json: Vec<serde_json::Value> = all_events
                .iter()
                .map(|e| {
                    let mut obj = serde_json::json!({
                        "type": e.event_type,
                        "severity": e.severity,
                    });
                    if let Some(ref code) = e.code {
                        obj["code"] = serde_json::json!(code);
                    }
                    obj["message"] = serde_json::json!(e.message);
                    if let Some(ref loc) = e.location {
                        obj["file"] = serde_json::json!(loc.file);
                        obj["line"] = serde_json::json!(loc.line);
                    }
                    obj
                })
                .collect();
            let json_out = serde_json::to_string_pretty(&events_json).unwrap();
            std::fs::write(json_path, json_out + "\n").unwrap();
            eprintln!("BLESSED: {} ({} events)", json_path.display(), all_events.len());
            return (
                0,
                0,
                FieldStats {
                    total: 0,
                    type_ok: 0,
                    severity_ok: 0,
                    code_ok: 0,
                    file_ok: 0,
                    line_ok: 0,
                },
            );
        }

        let json = std::fs::read_to_string(json_path).unwrap();
        let expected: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();

        let mut matched = 0;
        let mut stats = FieldStats {
            total: expected.len(),
            type_ok: 0,
            severity_ok: 0,
            code_ok: 0,
            file_ok: 0,
            line_ok: 0,
        };
        for (i, exp) in expected.iter().enumerate() {
            if i >= all_events.len() {
                break;
            }
            let event = &all_events[i];
            let type_ok = exp.get("type").is_none_or(|v| v.as_str() == Some(&event.event_type));
            let sev_ok =
                exp.get("severity").is_none_or(|v| v.as_str() == event.severity.as_deref());
            let code_ok = exp.get("code").is_none_or(|v| v.as_str() == event.code.as_deref());
            let file_ok = exp.get("file").is_none_or(|v| {
                event.location.as_ref().is_some_and(|loc| v.as_str() == Some(&loc.file))
            });
            let line_ok = exp.get("line").is_none_or(|v| {
                event.location.as_ref().is_some_and(|loc| v.as_u64() == Some(loc.line))
            });

            if type_ok {
                stats.type_ok += 1;
            }
            if sev_ok {
                stats.severity_ok += 1;
            }
            if code_ok {
                stats.code_ok += 1;
            }
            if file_ok {
                stats.file_ok += 1;
            }
            if line_ok {
                stats.line_ok += 1;
            }

            if type_ok && sev_ok && code_ok && file_ok && line_ok {
                matched += 1;
            }
        }
        (expected.len(), matched, stats)
    }

    /// Format per-field match rates for assertion messages.
    fn field_scores(stats: &FieldStats) -> String {
        if stats.total == 0 {
            return String::new();
        }
        format!(
            "type={:.0}% sev={:.0}% code={:.0}% file={:.0}% line={:.0}%",
            stats.type_ok as f64 / stats.total as f64 * 100.0,
            stats.severity_ok as f64 / stats.total as f64 * 100.0,
            stats.code_ok as f64 / stats.total as f64 * 100.0,
            stats.file_ok as f64 / stats.total as f64 * 100.0,
            stats.line_ok as f64 / stats.total as f64 * 100.0,
        )
    }

    fn run_parser_fixtures(parser_name: &str) {
        let base = std::path::Path::new("parsers/builtin/tests").join(parser_name);
        if !base.is_dir() {
            return;
        }

        let txt_files: Vec<_> = std::fs::read_dir(&base)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "txt"))
            .collect();

        assert!(!txt_files.is_empty(), "no fixtures in {}", base.display());

        for entry in &txt_files {
            let txt_path = entry.path();
            let json_path = txt_path.with_extension("json");
            let (total, matched, stats) = run_fixture(parser_name, &txt_path, &json_path);
            // Skip assertion in bless mode (total == 0 means we just wrote the JSON)
            if total == 0 {
                continue;
            }
            let score = matched as f64 / total as f64;
            let per_field = field_scores(&stats);
            assert!(
                score >= 0.95,
                "parser '{}' fixture '{}': {:.0}% ({} / {}) [{}]",
                parser_name,
                txt_path.file_stem().unwrap().to_str().unwrap(),
                score * 100.0,
                matched,
                total,
                per_field
            );
        }
    }

    #[test]
    fn fixture_tsc() {
        run_parser_fixtures("tsc");
    }

    #[test]
    fn fixture_cargo() {
        run_parser_fixtures("cargo");
    }

    #[test]
    fn fixture_jest() {
        run_parser_fixtures("jest");
    }

    #[test]
    fn fixture_eslint() {
        run_parser_fixtures("eslint");
    }

    #[test]
    fn fixture_go() {
        run_parser_fixtures("go");
    }

    #[test]
    fn fixture_python() {
        run_parser_fixtures("python");
    }

    #[test]
    fn fixture_webpack() {
        run_parser_fixtures("webpack");
    }

    #[test]
    fn fixture_cargo_test() {
        run_parser_fixtures("cargo-test");
    }

    #[test]
    fn fixture_cc() {
        run_parser_fixtures("cc");
    }

    #[test]
    fn fixture_clippy() {
        run_parser_fixtures("clippy");
    }

    #[test]
    fn fixture_esbuild() {
        run_parser_fixtures("esbuild");
    }

    #[test]
    fn fixture_gradle() {
        run_parser_fixtures("gradle");
    }

    #[test]
    fn fixture_make() {
        run_parser_fixtures("make");
    }

    #[test]
    fn fixture_mocha() {
        run_parser_fixtures("mocha");
    }

    #[test]
    fn fixture_npm() {
        run_parser_fixtures("npm");
    }

    #[test]
    fn fixture_pip() {
        run_parser_fixtures("pip");
    }

    #[test]
    fn fixture_pnpm() {
        run_parser_fixtures("pnpm");
    }

    #[test]
    fn fixture_prettier() {
        run_parser_fixtures("prettier");
    }

    #[test]
    fn fixture_swc() {
        run_parser_fixtures("swc");
    }

    #[test]
    fn fixture_vite() {
        run_parser_fixtures("vite");
    }

    #[test]
    fn fixture_docker() {
        run_parser_fixtures("docker");
    }

    #[test]
    fn fixture_aws() {
        run_parser_fixtures("aws");
    }

    #[test]
    fn fixture_terraform() {
        run_parser_fixtures("terraform");
    }

    #[test]
    fn fixture_kubectl() {
        run_parser_fixtures("kubectl");
    }

    #[test]
    fn fixture_helm() {
        run_parser_fixtures("helm");
    }

    #[test]
    fn fixture_ruff() {
        run_parser_fixtures("ruff");
    }

    #[test]
    fn fixture_uv() {
        run_parser_fixtures("uv");
    }

    #[test]
    fn fixture_bun() {
        run_parser_fixtures("bun");
    }

    #[test]
    fn fixture_deno() {
        run_parser_fixtures("deno");
    }

    #[test]
    fn fixture_nx() {
        run_parser_fixtures("nx");
    }

    #[test]
    fn fixture_turbo() {
        run_parser_fixtures("turbo");
    }

    #[test]
    fn fixture_biome() {
        run_parser_fixtures("biome");
    }

    #[test]
    fn fixture_oxlint() {
        run_parser_fixtures("oxlint");
    }

    #[test]
    fn fixture_vitest() {
        run_parser_fixtures("vitest");
    }

    #[test]
    fn all_parsers_load() {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();

        let parsers = [
            "tsc",
            "cargo",
            "jest",
            "vite",
            "eslint",
            "go",
            "python",
            "cc",
            "npm",
            "webpack",
            "prettier",
            "swc",
            "esbuild",
            "clippy",
            "make",
            "gradle",
            "cargo-test",
            "mocha",
            "pip",
            "pnpm",
            "terraform",
            "kubectl",
            "helm",
            "aws",
            "docker",
            "uv",
            "ruff",
            "turbo",
            "nx",
            "deno",
            "bun",
            "biome",
            "oxlint",
            "vitest",
            "git",
            "curl",
            "ssh",
        ];

        for name in &parsers {
            let tool = engine.detect(name);
            assert!(tool.is_some(), "parser '{}' not detected by engine", name);
        }
    }

    #[test]
    fn fixture_git() {
        run_parser_fixtures("git");
    }

    #[test]
    fn fixture_curl() {
        run_parser_fixtures("curl");
    }

    #[test]
    fn fixture_ssh() {
        run_parser_fixtures("ssh");
    }

    #[test]
    fn engine_reload_works() {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();
        // Manual reload should succeed without error
        engine.reload().unwrap();
        // Parsers should still be available after reload
        assert!(engine.detect("tsc").is_some());
    }
}

// ── Benchmark ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod benchmark {
    use super::*;
    use crate::ipc::TaskEvent;
    use std::path::Path;

    /// Parser names matching the builtin set (must match `all_parsers_load` list).
    const ALL_PARSERS: &[&str] = &[
        "tsc",
        "cargo",
        "jest",
        "vite",
        "eslint",
        "go",
        "python",
        "cc",
        "npm",
        "webpack",
        "prettier",
        "swc",
        "esbuild",
        "clippy",
        "make",
        "gradle",
        "cargo-test",
        "mocha",
        "pip",
        "pnpm",
        "terraform",
        "kubectl",
        "helm",
        "aws",
        "docker",
        "uv",
        "ruff",
        "turbo",
        "nx",
        "deno",
        "bun",
        "biome",
        "oxlint",
        "vitest",
        "git",
        "curl",
        "ssh",
    ];

    /// Results for a single fixture file.
    #[derive(serde::Serialize)]
    struct FixtureResult {
        parser: String,
        fixture: String,
        /// Number of non-empty lines in the raw input.
        raw_lines: usize,
        /// Number of structured events produced.
        events: usize,
        /// Word count of raw input text.
        raw_tokens: usize,
        /// Word count of structured JSON output.
        structured_tokens: usize,
        /// raw_tokens / structured_tokens.
        compression_ratio: f64,
        /// Total actionable fields in structured output (type+severity+code+file+line+message per event).
        structured_fields: usize,
        /// Average actionable fields per event.
        fields_per_event: f64,
        /// 0-indexed line number of first error/warning in raw text.
        raw_first_error_line: Option<usize>,
        /// Index of first error/warning event in structured output.
        structured_first_error_idx: Option<usize>,
        /// Whether structured output finds the error faster (lower index).
        error_faster: Option<bool>,
        /// Parser accuracy: matched events / expected events.
        accuracy: f64,
        /// Per-field accuracy percentages.
        field_accuracy: FieldAcc,
        /// Events with file:line location (structured advantage over raw text).
        events_with_location: usize,
        /// Events with error/warning code extracted.
        events_with_code: usize,
        /// Diagnostic events (not raw log fallback).
        diagnostic_events: usize,
    }

    #[derive(serde::Serialize)]
    struct FieldAcc {
        r#type: f64,
        severity: f64,
        code: f64,
        file: f64,
        line: f64,
    }

    /// Aggregate results across all fixtures.
    #[derive(serde::Serialize)]
    struct BenchmarkResult {
        total_fixtures: usize,
        total_raw_lines: usize,
        total_events: usize,
        total_raw_tokens: usize,
        total_structured_tokens: usize,
        /// Overall compression ratio (total_raw / total_structured).
        compression_ratio: f64,
        /// Total actionable fields across all structured events.
        total_structured_fields: usize,
        /// Average actionable fields per event across all fixtures.
        avg_fields_per_event: f64,
        /// Percentage of fixtures where structured output locates errors faster.
        error_speed_advantage_pct: f64,
        /// Average parser accuracy across all fixtures.
        avg_accuracy: f64,
        /// Total events with file:line location across all fixtures.
        total_events_with_location: usize,
        /// Total events with error code extracted.
        total_events_with_code: usize,
        /// Total diagnostic events (not raw log fallback).
        total_diagnostic_events: usize,
        /// Per-parser results.
        details: Vec<FixtureResult>,
    }

    /// Count tokens (words) in a string — whitespace-split approximation.
    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    /// Find the 0-indexed line number of the first error or warning in raw text.
    fn find_first_error_line(lines: &[&str]) -> Option<usize> {
        let error_patterns =
            ["error", "Error", "ERROR", "warning", "Warning", "WARNING", "panic", "fatal", "FAIL"];
        for (i, line) in lines.iter().enumerate() {
            if error_patterns.iter().any(|p| line.contains(p)) {
                return Some(i);
            }
        }
        None
    }

    /// Find the index of the first error/warning event in structured output.
    fn find_first_error_event(events: &[TaskEvent]) -> Option<usize> {
        events.iter().position(|e| {
            e.severity.as_deref() == Some("error") || e.severity.as_deref() == Some("warning")
        })
    }

    /// Compute field-level accuracy against expected JSON (same logic as harness_tests).
    fn compute_accuracy(
        events: &[TaskEvent],
        expected: &[serde_json::Value],
    ) -> (usize, usize, (usize, usize, usize, usize, usize)) {
        let mut matched = 0;
        let (mut t_ok, mut s_ok, mut c_ok, mut f_ok, mut l_ok) = (0, 0, 0, 0, 0);
        for (i, exp) in expected.iter().enumerate() {
            if i >= events.len() {
                break;
            }
            let event = &events[i];
            let type_ok = exp.get("type").is_none_or(|v| v.as_str() == Some(&event.event_type));
            let sev_ok =
                exp.get("severity").is_none_or(|v| v.as_str() == event.severity.as_deref());
            let code_ok = exp.get("code").is_none_or(|v| v.as_str() == event.code.as_deref());
            let file_ok = exp.get("file").is_none_or(|v| {
                event.location.as_ref().is_some_and(|loc| v.as_str() == Some(&loc.file))
            });
            let line_ok = exp.get("line").is_none_or(|v| {
                event.location.as_ref().is_some_and(|loc| v.as_u64() == Some(loc.line))
            });
            if type_ok {
                t_ok += 1;
            }
            if sev_ok {
                s_ok += 1;
            }
            if code_ok {
                c_ok += 1;
            }
            if file_ok {
                f_ok += 1;
            }
            if line_ok {
                l_ok += 1;
            }
            if type_ok && sev_ok && code_ok && file_ok && line_ok {
                matched += 1;
            }
        }
        (expected.len(), matched, (t_ok, s_ok, c_ok, f_ok, l_ok))
    }

    fn run_fixture_bench(
        engine: &Engine,
        parser_name: &str,
        txt_path: &Path,
        json_path: &Path,
    ) -> FixtureResult {
        let tool = engine.get_by_name(parser_name).or_else(|| engine.detect(parser_name));
        let session = engine.create_session(tool.as_ref());

        let txt = std::fs::read_to_string(txt_path).unwrap();
        let raw_lines: Vec<&str> = txt.lines().filter(|l| !l.trim().is_empty()).collect();

        let mut events: Vec<TaskEvent> = Vec::new();
        for (seq, line) in raw_lines.iter().enumerate() {
            events.extend(session.parse_line(line, seq as u64, tool.as_ref()));
        }

        let raw_tokens = count_tokens(&txt);
        let structured_json = serde_json::to_string(&events).unwrap();
        let structured_tokens = count_tokens(&structured_json);

        let compression_ratio =
            if structured_tokens > 0 { raw_tokens as f64 / structured_tokens as f64 } else { 0.0 };

        let raw_first_error_line = find_first_error_line(&raw_lines);
        let structured_first_error_idx = find_first_error_event(&events);

        let error_faster = match (raw_first_error_line, structured_first_error_idx) {
            (Some(raw), Some(structured)) => Some(structured < raw),
            (None, Some(_)) => Some(true), // structured found one, raw didn't
            _ => None,
        };

        // Accuracy against expected JSON
        let json = std::fs::read_to_string(json_path).unwrap();
        let expected: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        let (total, matched, (t, s, c, f, l)) = compute_accuracy(&events, &expected);
        let accuracy = if total > 0 { matched as f64 / total as f64 } else { 1.0 };
        let field_accuracy = FieldAcc {
            r#type: if total > 0 { t as f64 / total as f64 } else { 1.0 },
            severity: if total > 0 { s as f64 / total as f64 } else { 1.0 },
            code: if total > 0 { c as f64 / total as f64 } else { 1.0 },
            file: if total > 0 { f as f64 / total as f64 } else { 1.0 },
            line: if total > 0 { l as f64 / total as f64 } else { 1.0 },
        };

        // Information density: count actionable fields per event
        // (type + severity + code + file + line + message = 6 max per event)
        let structured_fields: usize = events
            .iter()
            .map(|e| {
                let mut count = 2; // type + message are always present
                if e.severity.is_some() {
                    count += 1;
                }
                if e.code.is_some() {
                    count += 1;
                }
                if e.location.is_some() {
                    count += 1; // file + line (always together)
                }
                count
            })
            .sum();
        let fields_per_event =
            if !events.is_empty() { structured_fields as f64 / events.len() as f64 } else { 0.0 };

        // Feature value metrics
        let events_with_location = events.iter().filter(|e| e.location.is_some()).count();
        let events_with_code = events.iter().filter(|e| e.code.is_some()).count();
        let diagnostic_events = events.iter().filter(|e| e.event_type != "log").count();

        FixtureResult {
            parser: parser_name.to_string(),
            fixture: txt_path.file_stem().unwrap().to_str().unwrap().to_string(),
            raw_lines: raw_lines.len(),
            events: events.len(),
            raw_tokens,
            structured_tokens,
            compression_ratio,
            structured_fields,
            fields_per_event,
            raw_first_error_line,
            structured_first_error_idx,
            error_faster,
            accuracy,
            field_accuracy,
            events_with_location,
            events_with_code,
            diagnostic_events,
        }
    }

    #[test]
    fn run_benchmark() {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();

        let mut all_results: Vec<FixtureResult> = Vec::new();

        for parser_name in ALL_PARSERS {
            let base = Path::new("parsers/builtin/tests").join(parser_name);
            if !base.is_dir() {
                continue;
            }
            let txt_files: Vec<_> = std::fs::read_dir(&base)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "txt"))
                .collect();

            for entry in &txt_files {
                let txt_path = entry.path();
                let json_path = txt_path.with_extension("json");
                let result = run_fixture_bench(&engine, parser_name, &txt_path, &json_path);
                all_results.push(result);
            }
        }

        // Aggregate
        let total_fixtures = all_results.len();
        let total_raw_lines = all_results.iter().map(|r| r.raw_lines).sum();
        let total_events = all_results.iter().map(|r| r.events).sum();
        let total_raw_tokens = all_results.iter().map(|r| r.raw_tokens).sum();
        let total_structured_tokens: usize = all_results.iter().map(|r| r.structured_tokens).sum();
        let total_structured_fields: usize = all_results.iter().map(|r| r.structured_fields).sum();
        let compression_ratio = if total_structured_tokens > 0 {
            total_raw_tokens as f64 / total_structured_tokens as f64
        } else {
            0.0
        };
        let avg_fields_per_event = if total_events > 0 {
            total_structured_fields as f64 / total_events as f64
        } else {
            0.0
        };

        let error_speed_results: Vec<_> =
            all_results.iter().filter(|r| r.error_faster.is_some()).collect();
        let error_speed_advantage_pct = if !error_speed_results.is_empty() {
            error_speed_results.iter().filter(|r| r.error_faster.unwrap()).count() as f64
                / error_speed_results.len() as f64
                * 100.0
        } else {
            0.0
        };

        let avg_accuracy = if !all_results.is_empty() {
            all_results.iter().map(|r| r.accuracy).sum::<f64>() / all_results.len() as f64
        } else {
            0.0
        };

        let total_events_with_location: usize =
            all_results.iter().map(|r| r.events_with_location).sum();
        let total_events_with_code: usize = all_results.iter().map(|r| r.events_with_code).sum();
        let total_diagnostic_events: usize = all_results.iter().map(|r| r.diagnostic_events).sum();

        let bench = BenchmarkResult {
            total_fixtures,
            total_raw_lines,
            total_events,
            total_raw_tokens,
            total_structured_tokens,
            compression_ratio,
            total_structured_fields,
            avg_fields_per_event,
            error_speed_advantage_pct,
            avg_accuracy,
            total_events_with_location,
            total_events_with_code,
            total_diagnostic_events,
            details: all_results,
        };

        // Output structured JSON to stderr (captured by --nocapture)
        let json = serde_json::to_string_pretty(&bench).unwrap();
        eprintln!("\n=== ARSHY BENCHMARK ===\n{}\n=== END BENCHMARK ===", json);
    }
}
