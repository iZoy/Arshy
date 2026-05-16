//! TOML parser definition format — deserializes `.toml` parser files
//! and converts to internal `LinePattern` / `StatefulPattern` types.
//!
//! # Format
//!
//! ```toml
//! [meta]
//! name = "tsc"
//! description = "TypeScript compiler"
//! detect = ["tsc"]
//! parser_type = "toml"        # "toml" (default) or "stateful"
//! priority = 50
//!
//! [[pattern]]
//! name = "ts-error"
//! regex = '^(.+?)\((\d+)\): (.+)$'
//! event_type = "diagnostic"
//! severity = "error"
//! fields = { file = 1, line = 2, message = 3 }
//! # stateful-only fields:
//! state_condition = "state=value"   # optional
//! state_transition = "key=value"    # optional
//! ```

use std::collections::HashMap;

use serde::Deserialize;

use super::rhai::StatefulPattern;
use super::toml::LinePattern;

// ── TOML schema types ────────────────────────────────────────────────────────

/// Top-level parser definition deserialized from a `.toml` file.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TomlParserDef {
    pub meta: MetaDef,
    #[serde(rename = "pattern")]
    pub patterns: Vec<PatternDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetaDef {
    pub name: String,
    pub description: String,
    pub detect: Vec<String>,
    /// Patterns matched against the full command (not just first word).
    /// Used for multi-word commands like "cargo test".
    #[serde(default)]
    pub detect_full: Vec<String>,
    /// "toml" (default) or "stateful"
    pub parser_type: String,
    pub priority: u32,
    pub min_version: Option<String>,
    pub max_version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PatternDef {
    pub name: String,
    pub regex: String,
    pub event_type: String,
    pub severity: String,
    /// Maps field names → regex capture group indices.
    /// Special key "severity" maps to a capture group whose value overrides `severity`.
    #[serde(default)]
    pub fields: HashMap<String, usize>,
    /// Stateful-only: condition for this pattern to match.
    /// Format: "key=value" — only matches when state key equals value.
    pub state_condition: Option<String>,
    /// Stateful-only: state transition after matching.
    /// Format: "key=value" — sets state key to value.
    pub state_transition: Option<String>,
}

// ── Defaults ─────────────────────────────────────────────────────────────────

impl Default for TomlParserDef {
    fn default() -> Self {
        Self {
            meta: MetaDef {
                parser_type: "toml".into(),
                priority: 50,
                ..Default::default()
            },
            patterns: Vec::new(),
        }
    }
}

// ── Conversion to internal types ─────────────────────────────────────────────

impl TomlParserDef {
    /// Parse a TOML string into a `TomlParserDef`.
    pub fn parse(content: &str) -> Result<Self, String> {
        toml::from_str(content).map_err(|e| format!("TOML parse error: {}", e))
    }

    /// Whether this is a stateful (Rhai-type) parser.
    pub fn is_stateful(&self) -> bool {
        self.meta.parser_type == "stateful"
    }

    /// Convert all pattern defs to `LinePattern` (for TOML/stateless parsers).
    /// Skips patterns with invalid regex, logging warnings.
    pub fn to_line_patterns(&self) -> Vec<LinePattern> {
        self.patterns
            .iter()
            .filter_map(|p| match p.to_line_pattern() {
                Ok(pat) => Some(pat),
                Err(e) => {
                    tracing::warn!("parser '{}': skipping pattern '{}': {}", self.meta.name, p.name, e);
                    None
                }
            })
            .collect()
    }

    /// Convert all pattern defs to `StatefulPattern` (for stateful parsers).
    /// Skips patterns with invalid regex, logging warnings.
    pub fn to_stateful_patterns(&self) -> Vec<StatefulPattern> {
        self.patterns
            .iter()
            .filter_map(|p| match p.to_stateful_pattern() {
                Ok(pat) => Some(pat),
                Err(e) => {
                    tracing::warn!("parser '{}': skipping stateful pattern '{}': {}", self.meta.name, p.name, e);
                    None
                }
            })
            .collect()
    }
}

impl PatternDef {
    /// Convert to a `LinePattern` (stateless, line-by-line matching).
    fn to_line_pattern(&self) -> Result<LinePattern, String> {
        let regex = regex::Regex::new(&self.regex)
            .map_err(|e| format!("invalid regex '{}': {}", self.regex, e))?;

        Ok(LinePattern {
            regex,
            event_type: self.event_type.clone(),
            severity: self.severity.clone(),
            file_group: self.fields.get("file").copied(),
            line_group: self.fields.get("line").copied(),
            col_group: self.fields.get("column").copied(),
            code_group: self.fields.get("code").copied(),
            message_group: self.fields.get("message").copied(),
        })
    }

    /// Convert to a `StatefulPattern` (cross-line state machine matching).
    fn to_stateful_pattern(&self) -> Result<StatefulPattern, String> {
        let regex = regex::Regex::new(&self.regex)
            .map_err(|e| format!("invalid regex '{}': {}", self.regex, e))?;

        let state_condition = self.state_condition.as_deref().and_then(parse_key_value);
        let state_transition = self.state_transition.as_deref().and_then(parse_key_value);

        Ok(StatefulPattern {
            regex,
            event_type: self.event_type.clone(),
            severity: self.severity.clone(),
            message_group: self.fields.get("message").copied(),
            file_group: self.fields.get("file").copied(),
            line_group: self.fields.get("line").copied(),
            state_condition,
            state_transition,
        })
    }
}

/// Parse "key=value" into a `(String, String)` tuple.
/// Returns `None` if the format is invalid.
fn parse_key_value(s: &str) -> Option<(String, String)> {
    let (key, value) = s.split_once('=')?;
    if key.is_empty() || value.is_empty() {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

// ── Built-in parser loading ──────────────────────────────────────────────────

/// Embedded builtin parser TOML definitions.
/// Each entry is `(filename, content)` where filename is used for logging.
pub const BUILTIN_TOML: &[(&str, &str)] = &[
    // Tier 1: Original 10 parsers
    ("tsc.toml", include_str!("../../../parsers/builtin/tsc.toml")),
    ("cargo.toml", include_str!("../../../parsers/builtin/cargo.toml")),
    ("jest.toml", include_str!("../../../parsers/builtin/jest.toml")),
    ("vite.toml", include_str!("../../../parsers/builtin/vite.toml")),
    ("eslint.toml", include_str!("../../../parsers/builtin/eslint.toml")),
    ("go.toml", include_str!("../../../parsers/builtin/go.toml")),
    ("python.toml", include_str!("../../../parsers/builtin/python.toml")),
    ("cc.toml", include_str!("../../../parsers/builtin/cc.toml")),
    ("npm.toml", include_str!("../../../parsers/builtin/npm.toml")),
    ("webpack.toml", include_str!("../../../parsers/builtin/webpack.toml")),
    // Tier 2: M2 expansion — 10 additional parsers
    ("prettier.toml", include_str!("../../../parsers/builtin/prettier.toml")),
    ("swc.toml", include_str!("../../../parsers/builtin/swc.toml")),
    ("esbuild.toml", include_str!("../../../parsers/builtin/esbuild.toml")),
    ("clippy.toml", include_str!("../../../parsers/builtin/clippy.toml")),
    ("make.toml", include_str!("../../../parsers/builtin/make.toml")),
    ("gradle.toml", include_str!("../../../parsers/builtin/gradle.toml")),
    ("cargo-test.toml", include_str!("../../../parsers/builtin/cargo-test.toml")),
    ("mocha.toml", include_str!("../../../parsers/builtin/mocha.toml")),
    ("pip.toml", include_str!("../../../parsers/builtin/pip.toml")),
    ("pnpm.toml", include_str!("../../../parsers/builtin/pnpm.toml")),
];

/// Load all builtin parser definitions.
/// Returns `(filename, def)` pairs. Logs warnings for parse failures.
pub fn load_builtins() -> Vec<(&'static str, TomlParserDef)> {
    BUILTIN_TOML
        .iter()
        .filter_map(|(name, content)| match TomlParserDef::parse(content) {
            Ok(def) => Some((*name, def)),
            Err(e) => {
                tracing::error!("builtin parser '{}': {}", name, e);
                None
            }
        })
        .collect()
}

/// Load a parser definition from a filesystem path.
pub fn load_from_path(path: &std::path::Path) -> Option<TomlParserDef> {
    let content = std::fs::read_to_string(path).ok()?;
    match TomlParserDef::parse(&content) {
        Ok(def) => Some(def),
        Err(e) => {
            tracing::warn!("parser file '{}': {}", path.display(), e);
            None
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tsc_toml() {
        let def = TomlParserDef::parse(include_str!("../../../parsers/builtin/tsc.toml")).unwrap();
        assert_eq!(def.meta.name, "tsc");
        assert_eq!(def.meta.detect, vec!["tsc"]);
        assert_eq!(def.patterns.len(), 4);
        assert!(!def.is_stateful());

        let patterns = def.to_line_patterns();
        assert_eq!(patterns.len(), 4);
        assert_eq!(patterns[0].event_type, "diagnostic");
        assert_eq!(patterns[0].severity, "error");
        assert_eq!(patterns[0].file_group, Some(1));
        assert_eq!(patterns[0].line_group, Some(2));
    }

    #[test]
    fn parse_npm_toml_stateful() {
        let def = TomlParserDef::parse(include_str!("../../../parsers/builtin/npm.toml")).unwrap();
        assert_eq!(def.meta.name, "npm");
        assert!(def.is_stateful());
        assert_eq!(def.patterns.len(), 5);

        let patterns = def.to_stateful_patterns();
        assert_eq!(patterns.len(), 5);
        // First pattern has state_transition
        assert!(patterns[0].state_transition.is_some());
        assert_eq!(patterns[0].state_transition.as_ref().unwrap().0, "packages_added");
    }

    #[test]
    fn parse_all_builtins() {
        let builtins = load_builtins();
        assert_eq!(builtins.len(), 20, "expected 20 builtin parsers");

        for (name, def) in &builtins {
            assert!(!def.meta.name.is_empty(), "parser '{}' has empty name", name);
            assert!(!def.patterns.is_empty(), "parser '{}' has no patterns", name);
        }
    }

    #[test]
    fn parse_key_value_works() {
        assert_eq!(parse_key_value("key=value"), Some(("key".into(), "value".into())));
        assert_eq!(parse_key_value("packages_added=done"), Some(("packages_added".into(), "done".into())));
        assert_eq!(parse_key_value(""), None);
        assert_eq!(parse_key_value("novalue"), None);
        assert_eq!(parse_key_value("=empty_key"), None);
    }

    #[test]
    fn invalid_regex_skipped() {
        let toml = r#"
[meta]
name = "test"

[[pattern]]
name = "bad"
regex = '(unclosed'
event_type = "diagnostic"
severity = "error"

[[pattern]]
name = "good"
regex = '^(.+)$'
event_type = "log"
severity = "info"
"#;
        let def = TomlParserDef::parse(toml).unwrap();
        let patterns = def.to_line_patterns();
        assert_eq!(patterns.len(), 1, "invalid regex should be skipped");
        assert_eq!(patterns[0].event_type, "log");
    }
}
