//! Parser registry — loads, deduplicates, and serves parser definitions.
//!
//! Sources (in priority order):
//! 1. Builtin parsers (embedded TOML, compiled into binary)
//! 2. User parsers (filesystem, `~/.arshy/parsers/*.toml` / `*.rhai`)
//!
//! Same-name user parsers override builtins.

use arshy_lib::config::ParserConfig;
use arshy_lib::Result;

use super::rhai::StatefulPattern;
use super::toml::LinePattern;
use super::toml_def;
use super::{ParsedTool, ParserType};

/// A single parser entry in the registry.
///
/// Holds both the metadata (for detection) and the compiled patterns (for parsing).
#[derive(Debug, Clone)]
pub struct ParserEntry {
    pub name: String,
    pub tool_name: String,
    pub detect_patterns: Vec<String>,
    /// Patterns matched against the full command (for multi-word commands).
    pub detect_full_patterns: Vec<String>,
    pub parser_type: ParserType,
    pub source: ParserSource,
    pub priority: u32,
    /// Compiled line patterns (for TOML parsers). Empty for stateful-only parsers.
    pub line_patterns: Vec<LinePattern>,
    /// Compiled stateful patterns (for stateful parsers). Empty for TOML-only parsers.
    pub stateful_patterns: Vec<StatefulPattern>,
    /// Rhai script source (for `.rhai` user parsers). None for TOML parsers.
    pub rhai_script: Option<String>,
    /// Minimum tool version required (semver, inclusive). None = no minimum.
    pub min_version: Option<String>,
    /// Maximum tool version supported (semver, inclusive). None = no maximum.
    pub max_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq)]
pub enum ParserSource {
    Builtin,
    User,
}

/// Registry holding all available parsers, sorted by priority.
pub struct ParserRegistry {
    entries: Vec<ParserEntry>,
}

impl ParserRegistry {
    /// Load all parsers: builtin (embedded TOML) + from configured filesystem dirs.
    pub fn load(config: &ParserConfig) -> Result<Self> {
        let mut entries = Vec::new();

        // 1. Load builtin parsers from embedded TOML definitions
        for (_filename, def) in toml_def::load_builtins() {
            let entry = def_to_entry(def, ParserSource::Builtin);
            entries.push(entry);
        }

        // Raw fallback — always present, lowest priority, never auto-detected.
        // Used as the final fallback when no other parser matches.
        entries.push(ParserEntry {
            name: "raw".into(),
            tool_name: "*".into(),
            detect_patterns: vec![],
            detect_full_patterns: vec![],
            parser_type: ParserType::Raw,
            source: ParserSource::Builtin,
            priority: 0,
            line_patterns: vec![],
            stateful_patterns: vec![],
            rhai_script: None,
            min_version: None,
            max_version: None,
        });

        // 2. Load user parsers from filesystem directories
        for dir in &config.dirs {
            let expanded = arshy_lib::config::expand_path(dir);
            if expanded.is_dir() {
                if let Ok(files) = std::fs::read_dir(&expanded) {
                    for file_entry in files.flatten() {
                        let path = file_entry.path();
                        if let Some(entry) = load_user_parser(&path) {
                            entries.push(entry);
                        }
                    }
                }
            }
        }

        // Sort: higher priority first, user beats builtin on tie
        entries.sort_by(|a, b| {
            b.priority.cmp(&a.priority)
                .then_with(|| a.source.cmp(&b.source))
        });

        // Dedup by name — first occurrence wins (higher priority / user source)
        let mut seen = std::collections::HashSet::new();
        entries.retain(|e| seen.insert(e.name.clone()));

        Ok(Self { entries })
    }

    /// Find the best parser matching a command.
    ///
    /// Detection (checked in priority order, first match wins):
    /// 1. `detect_full` patterns — matched against full command via `starts_with`
    ///    (e.g. `"cargo test"` matches `"cargo test -- --test-threads=1"`)
    /// 2. `detect` patterns — matched against first word via `starts_with`
    ///    (e.g. `"cargo"` matches `"cargo build"`)
    /// 3. For chained commands (`&&`, `||`, `;`) — retry with each segment's
    ///    first word (e.g. `"echo x && cargo build"` → detects `"cargo"`)
    ///
    /// `starts_with` avoids false positives that `contains` had
    /// (e.g. "pnpm" no longer matches npm's "npm" pattern).
    pub fn detect(&self, command: &str) -> Option<ParsedTool> {
        let cmd_name = command.split_whitespace().next()?.to_lowercase();
        let cmd_lower = command.to_lowercase();

        // Pass 1: full command + first word (standard detection)
        if let Some(tool) = self.try_detect(&cmd_lower, &cmd_name) {
            return Some(tool);
        }

        // Pass 2: chained command — split on &&, ||, ; and retry each segment
        let has_chain = cmd_lower.contains("&&")
            || cmd_lower.contains("||")
            || cmd_lower.contains(';');
        if has_chain {
            let segments: Vec<&str> = cmd_lower
                .split("&&")
                .flat_map(|s| s.split("||"))
                .flat_map(|s| s.split(';'))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            for segment in segments {
                let seg_name = segment.split_whitespace().next()?;
                if seg_name == cmd_name {
                    continue; // already tried
                }
                if let Some(tool) = self.try_detect(&cmd_lower, seg_name) {
                    return Some(tool);
                }
            }
        }

        None
    }

    /// Try to match a command against all registry entries using the given
    /// first-word hint. Returns the first matching parsed tool, or None.
    fn try_detect(&self, cmd_lower: &str, first_word: &str) -> Option<ParsedTool> {
        for entry in &self.entries {
            if entry.parser_type == ParserType::Raw {
                continue;
            }
            // Check detect_full patterns first (matched against full command)
            for pat in &entry.detect_full_patterns {
                if cmd_lower.starts_with(&pat.to_lowercase()) {
                    return Some(ParsedTool {
                        tool_name: entry.tool_name.clone(),
                        parser_name: entry.name.clone(),
                        parser_type: entry.parser_type.clone(),
                        version: None,
                    });
                }
            }
            // Then check detect patterns (matched against first word)
            for pat in &entry.detect_patterns {
                if first_word.starts_with(&pat.to_lowercase()) {
                    return Some(ParsedTool {
                        tool_name: entry.tool_name.clone(),
                        parser_name: entry.name.clone(),
                        parser_type: entry.parser_type.clone(),
                        version: None,
                    });
                }
            }
        }
        None
    }

    /// Get a parser entry by name. Used by the engine to access patterns.
    pub fn get(&self, name: &str) -> Option<&ParserEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// Convert a parsed TOML definition into a registry entry.
fn def_to_entry(def: toml_def::TomlParserDef, source: ParserSource) -> ParserEntry {
    let is_stateful = def.is_stateful();

    ParserEntry {
        name: def.meta.name.clone(),
        tool_name: def.meta.name.clone(),
        detect_patterns: def.meta.detect.clone(),
        detect_full_patterns: def.meta.detect_full.clone(),
        parser_type: if is_stateful { ParserType::Rhai } else { ParserType::Toml },
        source,
        priority: def.meta.priority,
        line_patterns: if is_stateful { Vec::new() } else { def.to_line_patterns() },
        stateful_patterns: if is_stateful { def.to_stateful_patterns() } else { Vec::new() },
        rhai_script: None,
        min_version: def.meta.min_version.clone(),
        max_version: def.meta.max_version.clone(),
    }
}

/// Load a user parser from a filesystem path.
/// Supports `.toml` (declarative) and `.rhai` (stateful) files.
fn load_user_parser(path: &std::path::Path) -> Option<ParserEntry> {
    let ext = path.extension()?.to_str()?;
    let stem = path.file_stem()?.to_str()?;

    match ext {
        "toml" => {
            let def = toml_def::load_from_path(path)?;
            let mut entry = def_to_entry(def, ParserSource::User);
            // If TOML has no detect patterns, default to filename stem
            if entry.detect_patterns.is_empty() {
                entry.detect_patterns = vec![stem.to_string()];
            }
            Some(entry)
        }
        "rhai" => {
            // Load rhai script content from filesystem.
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("cannot read rhai file '{}': {}", path.display(), e);
                    return None;
                }
            };
            // Validate syntax before registering.
            if let Err(e) = rhai::Engine::new().compile(&content) {
                tracing::warn!("rhai syntax error in '{}': {}", path.display(), e);
                return None;
            }
            Some(ParserEntry {
                name: stem.to_string(),
                tool_name: stem.to_string(),
                detect_patterns: vec![stem.to_string()],
                detect_full_patterns: vec![],
                parser_type: ParserType::Rhai,
                source: ParserSource::User,
                priority: 100, // user parsers get high priority
                line_patterns: Vec::new(),
                stateful_patterns: Vec::new(),
                rhai_script: Some(content),
                min_version: None,
                max_version: None,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_loads_all_parsers() {
        let config = ParserConfig::default();
        let registry = ParserRegistry::load(&config).unwrap();

        // Should have at least 10 builtin parsers (tsc, cargo, jest, vite, eslint,
        // go, python, cc, npm, webpack) + raw fallback
        let names: Vec<&str> = registry.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"tsc"), "missing tsc parser");
        assert!(names.contains(&"cargo"), "missing cargo parser");
        assert!(names.contains(&"npm"), "missing npm parser");
        assert!(names.contains(&"webpack"), "missing webpack parser");
        assert!(names.contains(&"raw"), "missing raw fallback");
    }

    #[test]
    fn test_detect_finds_tool() {
        let config = ParserConfig::default();
        let registry = ParserRegistry::load(&config).unwrap();

        let tool = registry.detect("tsc --noEmit").unwrap();
        assert_eq!(tool.tool_name, "tsc");
        assert_eq!(tool.parser_type, ParserType::Toml);

        let tool = registry.detect("npm install").unwrap();
        assert_eq!(tool.tool_name, "npm");
        assert_eq!(tool.parser_type, ParserType::Rhai);
    }

    #[test]
    fn test_detect_chained_command() {
        let config = ParserConfig::default();
        let registry = ParserRegistry::load(&config).unwrap();

        // Standard: first word is the tool
        let tool = registry.detect("cargo build").unwrap();
        assert_eq!(tool.tool_name, "cargo");

        // Chained with &&: second segment contains the tool
        let tool = registry.detect("echo hello && rustc file.rs 2>&1").unwrap();
        assert_eq!(tool.tool_name, "cargo"); // rustc maps to cargo parser

        // Chained with ||: second segment
        let tool = registry.detect("cat file || npm test -- --coverage").unwrap();
        assert_eq!(tool.tool_name, "npm");

        // Chained with ;: second segment
        let tool = registry.detect("ls -la; python3 -m pytest -v").unwrap();
        assert_eq!(tool.tool_name, "python");

        // No parser should match non-tool commands
        assert!(registry.detect("echo hello world").is_none());
        assert!(registry.detect("cd /tmp && ls").is_none());
    }

    #[test]
    fn test_detect_priority_order() {
        let config = ParserConfig::default();
        let registry = ParserRegistry::load(&config).unwrap();

        // "cargo test" should match cargo-test (detect_full), not cargo or go
        let tool = registry.detect("cargo test").unwrap();
        assert_eq!(tool.tool_name, "cargo-test");

        // "cargo build" should match cargo (detect prefix), not cargo-test
        let tool = registry.detect("cargo build").unwrap();
        assert_eq!(tool.tool_name, "cargo");
    }

    #[test]
    fn test_stateful_patterns_loaded() {
        let config = ParserConfig::default();
        let registry = ParserRegistry::load(&config).unwrap();

        let npm = registry.get("npm").unwrap();
        assert_eq!(npm.parser_type, ParserType::Rhai);
        assert!(!npm.stateful_patterns.is_empty(), "npm should have stateful patterns");
        assert!(npm.line_patterns.is_empty(), "npm should have no line patterns");
    }

    #[test]
    fn test_line_patterns_loaded() {
        let config = ParserConfig::default();
        let registry = ParserRegistry::load(&config).unwrap();

        let tsc = registry.get("tsc").unwrap();
        assert_eq!(tsc.parser_type, ParserType::Toml);
        assert!(!tsc.line_patterns.is_empty(), "tsc should have line patterns");
        assert!(tsc.stateful_patterns.is_empty(), "tsc should have no stateful patterns");
    }

    #[test]
    fn test_raw_fallback_always_present() {
        let config = ParserConfig::default();
        let registry = ParserRegistry::load(&config).unwrap();

        let raw = registry.get("raw").unwrap();
        assert_eq!(raw.parser_type, ParserType::Raw);
        assert_eq!(raw.priority, 0);
    }
}
