//! Error code -> fix suggestion lookup database.

use arshy_lib::ipc::{EventHint, RetryHint};
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
        let hint = db.lookup("rust", "E0308").expect("E0308 should exist");
        assert!(hint.cause.contains("Type mismatch"));
        assert!(hint.fix.is_some());
    }

    #[test]
    fn lookup_returns_none_for_unknown_code() {
        let db = HintDb::get();
        assert!(db.lookup("rust", "E99999").is_none());
        assert!(db.lookup("unknown", "E0308").is_none());
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
    fn lookup_no_retry_for_e0308() {
        let db = HintDb::get();
        let hint = db.lookup("rust", "E0308").expect("E0308 should exist");
        assert!(hint.retry.is_none(), "E0308 should not have retry suggestion");
    }

    #[test]
    fn lookup_no_retry_for_e0425() {
        let db = HintDb::get();
        let hint = db.lookup("rust", "E0425").expect("E0425 should exist");
        assert!(hint.retry.is_none(), "E0425 should not have retry suggestion");
    }
}
