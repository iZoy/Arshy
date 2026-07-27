//! Error code -> fix suggestion lookup database.

use crate::ipc::{EventHint, RetryHint};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ErrorFile {
    meta: ErrorFileMeta,
    error: Vec<ErrorEntry>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ErrorFileMeta {
    language: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ErrorEntry {
    code: String,
    cause: String,
    fix: Option<String>,
    retry_commands: Option<Vec<String>>,
    retry_reason: Option<String>,
}

#[allow(dead_code)]
static HINT_DB: LazyLock<HintDb> = LazyLock::new(HintDb::load);

#[allow(dead_code)]
pub struct HintDb {
    entries: HashMap<(String, String), EventHint>,
}

#[allow(dead_code)] // consumed by Task 3+ executor enrichment
impl HintDb {
    fn load() -> Self {
        let mut entries = HashMap::new();
        let files: &[(&str, &str)] = &[
            ("rust", include_str!("../../../parsers/errors/rust.toml")),
            ("typescript", include_str!("../../../parsers/errors/typescript.toml")),
            ("python", include_str!("../../../parsers/errors/python.toml")),
            ("go", include_str!("../../../parsers/errors/go.toml")),
        ];

        for &(language, content) in files {
            match toml::from_str::<ErrorFile>(content) {
                Ok(file) => {
                    for entry in file.error {
                        let retry = match (entry.retry_commands, entry.retry_reason) {
                            (Some(commands), Some(reason)) => Some(RetryHint { commands, reason }),
                            _ => None,
                        };
                        entries.insert(
                            (language.to_string(), entry.code),
                            EventHint { cause: entry.cause, fix: entry.fix, retry },
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("failed to parse errors/{}.toml: {}", language, e);
                }
            }
        }
        Self { entries }
    }

    pub fn get() -> &'static HintDb {
        &HINT_DB
    }

    pub fn lookup(&self, language: &str, code: &str) -> Option<&EventHint> {
        self.entries.get(&(language.to_string(), code.to_string()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[allow(dead_code)] // consumed by Task 3+ executor enrichment
pub fn tool_to_language(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "cargo" | "cargo-test" | "clippy" | "rustc" => Some("rust"),
        "tsc" | "eslint" | "biome" | "oxlint" | "prettier" | "swc" | "esbuild" | "vite"
        | "webpack" | "vitest" | "jest" | "mocha" => Some("typescript"),
        "python" | "pytest" | "ruff" | "pip" => Some("python"),
        "go" => Some("go"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_db_loads_without_error() {
        let db = HintDb::get();
        assert!(!db.is_empty());
    }

    #[test]
    fn lookup_returns_hint_for_known_code() {
        let db = HintDb::get();
        let hint = db.lookup("typescript", "TS2769").expect("TS2769 should exist");
        assert!(hint.cause.contains("overload"));
        assert!(hint.fix.is_some());
    }

    #[test]
    fn lookup_returns_none_for_unknown_code() {
        let db = HintDb::get();
        // Unknown code within a populated language
        assert!(db.lookup("typescript", "TS99999").is_none());
        // Unknown language
        assert!(db.lookup("unknown", "TS2769").is_none());
    }

    #[test]
    fn tool_to_language_maps_correctly() {
        assert_eq!(tool_to_language("cargo"), Some("rust"));
        assert_eq!(tool_to_language("tsc"), Some("typescript"));
        assert_eq!(tool_to_language("python"), Some("python"));
        assert_eq!(tool_to_language("go"), Some("go"));
        assert_eq!(tool_to_language("unknown_tool"), None);
    }

    #[test]
    fn hint_serialization_round_trip() {
        let hint = EventHint {
            cause: "Type mismatch".into(),
            fix: Some("Use .into()".into()),
            retry: Some(RetryHint {
                commands: vec!["cargo clean && cargo build".into()],
                reason: "Stale cache".into(),
            }),
        };
        let json = serde_json::to_string(&hint).unwrap();
        let back: EventHint = serde_json::from_str(&json).unwrap();
        assert_eq!(hint, back);
    }

    #[test]
    fn lookup_no_retry_for_ts2769() {
        let db = HintDb::get();
        let hint = db.lookup("typescript", "TS2769").expect("TS2769 should exist");
        assert!(hint.retry.is_none(), "TS2769 should not have retry suggestion");
    }

    #[test]
    fn lookup_returns_hint_for_typescript() {
        let db = HintDb::get();
        let hint = db.lookup("typescript", "TS2571").expect("TS2571 should exist");
        assert!(hint.cause.contains("unknown"));
        assert!(hint.fix.is_some());
    }

    #[test]
    fn lookup_returns_hint_for_python() {
        let db = HintDb::get();
        let hint = db.lookup("python", "SyntaxError").expect("SyntaxError should exist");
        assert!(hint.cause.contains("syntax"));
        assert!(hint.fix.is_some());
    }

    #[test]
    fn lookup_returns_hint_for_go() {
        let db = HintDb::get();
        let hint = db.lookup("go", "cannot assign").expect("cannot assign should exist");
        assert!(hint.cause.contains("Cannot assign"));
        assert!(hint.fix.is_some());
    }

    #[test]
    fn hint_db_contains_expected_languages() {
        let db = HintDb::get();
        // Verify each language has at least the expected codes
        assert!(db.lookup("typescript", "TS2769").is_some(), "TS missing");
        assert!(db.lookup("python", "SyntaxError").is_some(), "Python missing");
        assert!(db.lookup("go", "cannot assign").is_some(), "Go missing");
        // Rust has 0 codes (intentionally — agent already knows them)
        assert!(db.lookup("rust", "E0308").is_none(), "Rust should have no codes");
    }

    #[test]
    fn tool_to_language_covers_common_tools() {
        // Rust tools
        assert_eq!(tool_to_language("cargo"), Some("rust"));
        assert_eq!(tool_to_language("clippy"), Some("rust"));
        assert_eq!(tool_to_language("rustc"), Some("rust"));
        // TypeScript tools
        assert_eq!(tool_to_language("tsc"), Some("typescript"));
        assert_eq!(tool_to_language("eslint"), Some("typescript"));
        assert_eq!(tool_to_language("biome"), Some("typescript"));
        assert_eq!(tool_to_language("jest"), Some("typescript"));
        assert_eq!(tool_to_language("vite"), Some("typescript"));
        // Python tools
        assert_eq!(tool_to_language("python"), Some("python"));
        assert_eq!(tool_to_language("pytest"), Some("python"));
        assert_eq!(tool_to_language("ruff"), Some("python"));
        // Go tools
        assert_eq!(tool_to_language("go"), Some("go"));
        // Unknown
        assert_eq!(tool_to_language("kubectl"), None);
        assert_eq!(tool_to_language("docker"), None);
    }
}
