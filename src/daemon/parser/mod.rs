//! Parser engine — TOML (Tier 1) + Stateful (Tier 2) + Crash + Heuristic + Raw fallback.
//!
//! Coverage target: 70% TOML regex, 25% stateful, 5% crash/raw.
//!
//! Architecture:
//! - `toml_def` — TOML file format deserialization
//! - `toml` — stateless line-by-line matching
//! - `stateful` — stateful cross-line matching
//! - `crash` — universal crash/traceback detection
//! - `registry` — loads and deduplicates parser entries
//! - `detect` — tool extraction and version detection
//! - `loader` — filesystem hot-reload watcher (notify v7)

mod crash;
pub(crate) mod dedup;
mod detect;
mod heuristic;
mod json;
mod loader;
pub mod pair_merger;
mod redos;
mod registry;
pub mod stateful;
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
    /// Number of loaded parser entries (builtin + user).
    pub fn parser_count(&self) -> usize {
        self.registry.read().map(|r| r.count()).unwrap_or(0)
    }

    pub fn detect(&self, command: &str) -> Option<ParsedTool> {
        self.registry.read().ok()?.detect(command)
    }

    /// Look up a parser by name (for testing/harness use).
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
                    let stateful_patterns: Vec<stateful::StatefulPattern> = entry.stateful_patterns.iter()
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
                        Some(stateful::StatefulParser::with_patterns(
                            &entry.name,
                            stateful_patterns,
                        ))
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
    stateful: Option<stateful::StatefulParser>,
}

impl ParserSession {
    fn raw() -> Self {
        Self { toml_parser: None, stateful: None }
    }

    /// Parse one line of output.
    ///
    /// Pipeline order:
    /// 1. Format detection (JSON line) — highest priority for structured data
    /// 2. Stateful parser (state-machine patterns)
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
    Stateful,
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

        // Location events are handled by GenericPairMerger downstream.
        // If we just flushed a pending event, return it and re-buffer the
        // location so it comes out on the next feed()/finish() call.
        if event.event_type == "location" {
            if flushed.is_some() {
                self.pending = Some(event);
                return flushed;
            }
            return Some(event);
        }

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

    // Pattern 4: Caret/dash markers — `  ^^^`, `  ---`
    // Note: `-->` arrow markers are NOT absorbed here — they are parsed as
    // location events by the TOML parser and merged by GenericPairMerger.
    if trimmed.starts_with("^^^") || trimmed.starts_with("---") {
        return true;
    }

    false
}

// ── RustcContextMerger tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests;
