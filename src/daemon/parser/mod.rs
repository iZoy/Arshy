//! Parser engine — TOML (Tier 1) + Stateful (Tier 2) + Crash + Raw fallback.
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
mod detect;
mod json;
mod loader;
mod redos;
mod registry;
pub mod rhai;
pub mod toml;
pub mod toml_def;

pub use json::try_parse as try_parse_json;
pub use toml::stderr_looks_like_error;

pub use detect::*;
pub use registry::*;

use arshy_lib::config::ParserConfig;
use arshy_lib::ipc::TaskEvent;
use arshy_lib::Result;
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
        Ok(Self {
            registry: Arc::new(RwLock::new(registry)),
            config: config.clone(),
        })
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
                Ok(new_registry) => {
                    match registry.write() {
                        Ok(mut reg) => {
                            let audit = reg.diff(&new_registry);
                            tracing::info!("parser hot-reload: {}", audit);
                            *reg = new_registry;
                        }
                        Err(e) => tracing::error!("registry lock poisoned: {}", e),
                    }
                }
                Err(e) => tracing::error!("parser reload failed: {}", e),
            }
        })?;
        Ok(Some(watcher))
    }

    /// Reload parsers from disk (manual trigger).
    #[allow(dead_code)]
    pub fn reload(&self) -> Result<()> {
        let new_registry = ParserRegistry::load(&self.config)?;
        let mut reg = self.registry.write()
            .map_err(|_| arshy_lib::ArshyError::Other("registry lock poisoned".into()))?;
        let audit = reg.diff(&new_registry);
        tracing::info!("parser reload: {}", audit);
        *reg = new_registry;
        Ok(())
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
                            entry.name, entry.deprecated_count
                        );
                    }

                    // Filter line patterns: skip deprecated patterns that have a replacement
                    let line_patterns: Vec<toml::LinePattern> = entry.line_patterns.iter()
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
                        Some(rhai::StatefulParser::with_patterns(
                            &entry.name,
                            stateful_patterns,
                        ))
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
        Self {
            toml_parser: None,
            stateful: None,
        }
    }

    /// Parse one line of output.
    pub fn parse_line(&self, line: &str, seq: u64, _tool: Option<&ParsedTool>) -> Vec<TaskEvent> {
        // 1. Stateful parser (if it matched, return; otherwise fall through)
        if let Some(ref stateful) = self.stateful {
            let events = stateful.feed_line(line, seq);
            if !events.is_empty() {
                return events;
            }
            // Stateful parser didn't match — fall through to crash/raw
        }

        // 2. TOML parser (pre-built at session creation, no per-line clone)
        if let Some(ref parser) = self.toml_parser {
            if let Some(mut event) = parser.parse_line(line) {
                event.seq = seq;
                return vec![event];
            }
        }

        // 3. Crash parser (universal, no tool dependency)
        if let Some(mut event) = crash::try_parse_crash(line) {
            event.seq = seq;
            return vec![event];
        }

        // 4. Raw fallback
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

    fn run_fixture(parser_name: &str, txt_path: &std::path::Path, json_path: &std::path::Path) -> (usize, usize, FieldStats) {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();
        // Try direct name lookup first (parser name != command name for multi-word tools)
        let tool = engine.get_by_name(parser_name)
            .or_else(|| engine.detect(parser_name));
        let session = engine.create_session(tool.as_ref());

        let txt = std::fs::read_to_string(txt_path).unwrap();

        let lines: Vec<&str> = txt.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut all_events: Vec<TaskEvent> = Vec::new();
        for line in &lines {
            all_events.extend(session.parse_line(line, 0, tool.as_ref()));
        }

        // ── Bless mode: write actual output as expected JSON ──────────────
        if std::env::var("ARSHY_BLESS").is_ok() {
            let events_json: Vec<serde_json::Value> = all_events.iter().map(|e| {
                let mut obj = serde_json::json!({
                    "type": e.event_type,
                    "severity": e.severity,
                });
                if let Some(ref code) = e.code { obj["code"] = serde_json::json!(code); }
                obj["message"] = serde_json::json!(e.message);
                if let Some(ref loc) = e.location {
                    obj["file"] = serde_json::json!(loc.file);
                    obj["line"] = serde_json::json!(loc.line);
                }
                obj
            }).collect();
            let json_out = serde_json::to_string_pretty(&events_json).unwrap();
            std::fs::write(json_path, json_out + "\n").unwrap();
            eprintln!("BLESSED: {} ({} events)", json_path.display(), all_events.len());
            return (0, 0, FieldStats { total: 0, type_ok: 0, severity_ok: 0, code_ok: 0, file_ok: 0, line_ok: 0 });
        }

        let json = std::fs::read_to_string(json_path).unwrap();
        let expected: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();

        let mut matched = 0;
        let mut stats = FieldStats { total: expected.len(), type_ok: 0, severity_ok: 0, code_ok: 0, file_ok: 0, line_ok: 0 };
        for (i, exp) in expected.iter().enumerate() {
            if i >= all_events.len() {
                break;
            }
            let event = &all_events[i];
            let type_ok = exp.get("type").is_none_or(|v| v.as_str() == Some(&event.event_type));
            let sev_ok = exp.get("severity").is_none_or(|v| v.as_str() == event.severity.as_deref());
            let code_ok = exp.get("code").is_none_or(|v| v.as_str() == event.code.as_deref());
            let file_ok = exp.get("file").is_none_or(|v| {
                event.location.as_ref().is_some_and(|loc| v.as_str() == Some(&loc.file))
            });
            let line_ok = exp.get("line").is_none_or(|v| {
                event.location.as_ref().is_some_and(|loc| v.as_u64() == Some(loc.line))
            });

            if type_ok { stats.type_ok += 1; }
            if sev_ok { stats.severity_ok += 1; }
            if code_ok { stats.code_ok += 1; }
            if file_ok { stats.file_ok += 1; }
            if line_ok { stats.line_ok += 1; }

            if type_ok && sev_ok && code_ok && file_ok && line_ok {
                matched += 1;
            }
        }
        (expected.len(), matched, stats)
    }

    /// Format per-field match rates for assertion messages.
    fn field_scores(stats: &FieldStats) -> String {
        if stats.total == 0 { return String::new(); }
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
            let score = matched as f64 / total as f64;
            let per_field = field_scores(&stats);
            assert!(
                score >= 0.95,
                "parser '{}' fixture '{}': {:.0}% ({} / {}) [{}]",
                parser_name,
                txt_path.file_stem().unwrap().to_str().unwrap(),
                score * 100.0, matched, total, per_field
            );
        }
    }

    #[test]
    fn fixture_tsc() { run_parser_fixtures("tsc"); }

    #[test]
    fn fixture_cargo() { run_parser_fixtures("cargo"); }

    #[test]
    fn fixture_jest() { run_parser_fixtures("jest"); }

    #[test]
    fn fixture_eslint() { run_parser_fixtures("eslint"); }

    #[test]
    fn fixture_go() { run_parser_fixtures("go"); }

    #[test]
    fn fixture_python() { run_parser_fixtures("python"); }

    #[test]
    fn fixture_webpack() { run_parser_fixtures("webpack"); }

    #[test]
    fn fixture_cargo_test() { run_parser_fixtures("cargo-test"); }

    #[test]
    fn fixture_cc() { run_parser_fixtures("cc"); }

    #[test]
    fn fixture_clippy() { run_parser_fixtures("clippy"); }

    #[test]
    fn fixture_esbuild() { run_parser_fixtures("esbuild"); }

    #[test]
    fn fixture_gradle() { run_parser_fixtures("gradle"); }

    #[test]
    fn fixture_make() { run_parser_fixtures("make"); }

    #[test]
    fn fixture_mocha() { run_parser_fixtures("mocha"); }

    #[test]
    fn fixture_npm() { run_parser_fixtures("npm"); }

    #[test]
    fn fixture_pip() { run_parser_fixtures("pip"); }

    #[test]
    fn fixture_pnpm() { run_parser_fixtures("pnpm"); }

    #[test]
    fn fixture_prettier() { run_parser_fixtures("prettier"); }

    #[test]
    fn fixture_swc() { run_parser_fixtures("swc"); }

    #[test]
    fn fixture_vite() { run_parser_fixtures("vite"); }

    #[test]
    fn all_20_parsers_load() {
        let config = ParserConfig::default();
        let engine = Engine::new(&config).unwrap();

        let parsers = [
            "tsc", "cargo", "jest", "vite", "eslint", "go", "python", "cc",
            "npm", "webpack", "prettier", "swc", "esbuild", "clippy", "make",
            "gradle", "cargo-test", "mocha", "pip", "pnpm",
        ];

        for name in &parsers {
            let tool = engine.detect(name);
            assert!(tool.is_some(), "parser '{}' not detected by engine", name);
        }
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
