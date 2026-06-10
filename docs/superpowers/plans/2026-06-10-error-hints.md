# Error Code → Fix Suggestion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add deterministic error code → fix suggestion lookup. When a parser extracts an error code (E0308, TS2345, etc.), automatically attach a human-readable cause and fix hint to the event.

**Architecture:** TOML files in `parsers/errors/` store ~100 error codes across 4 languages. A `HintDb` loads them at startup via `include_str!`. Enrichment runs in the executor sync completion path, looking up hints for error events that have a `code` field.

**Tech Stack:** Rust, serde, toml, LazyLock, include_str!

**Spec:** `docs/superpowers/specs/2026-06-10-error-hints-design.md`

---

## File Structure

| File | Action | Purpose |
|------|--------|---------|
| `src/ipc/mod.rs` | Modify | Add `EventHint` struct + `hint` field on `TaskEvent` |
| `src/daemon/parser/hint.rs` | Create | `HintDb` with TOML loading + lookup |
| `src/daemon/parser/mod.rs` | Modify | Declare `pub mod hint;` |
| `src/daemon/exec/mod.rs` | Modify | Enrich events with hints in sync path |
| `parsers/errors/rust.toml` | Create | ~30 Rust error codes |
| `parsers/errors/typescript.toml` | Create | ~30 TypeScript error codes |
| `parsers/errors/python.toml` | Create | ~20 Python error codes |
| `parsers/errors/go.toml` | Create | ~20 Go error codes |

---

### Task 1: EventHint struct + HintDb

**Files:**
- Modify: `src/ipc/mod.rs` (add EventHint, add hint field to TaskEvent)
- Create: `src/daemon/parser/hint.rs`
- Modify: `src/daemon/parser/mod.rs` (declare module)

- [ ] **Step 1: Add EventHint to ipc/mod.rs**

In `src/ipc/mod.rs`, add new struct after `EventContext` (around line 162):

```rust
/// Fix suggestion attached to an error event via error code lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventHint {
    /// One-line explanation of what went wrong
    pub cause: String,
    /// Brief fix suggestion
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}
```

In the `TaskEvent` struct, add after the `context` field:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<EventHint>,
```

IMPORTANT: This changes TaskEvent serialization. All places that construct TaskEvent must add `hint: None`. Search for `TaskEvent {` across the codebase and add `hint: None` to each construction site. Key locations:
- `src/daemon/parser/toml.rs` — `raw_event()` function
- `src/daemon/parser/crash.rs` — `try_parse_crash()`
- `src/daemon/parser/heuristic.rs` — `build_event()`
- `src/daemon/parser/dedup.rs` — `flush()`
- `src/daemon/parser/json.rs` — JSON parsing
- Any test helpers that construct TaskEvent

- [ ] **Step 2: Create parsers/errors/ directory and a minimal TOML file**

Create `parsers/errors/rust.toml` with just 3 codes for testing:

```toml
[meta]
language = "rust"

[[error]]
code = "E0308"
cause = "Type mismatch — expected one type but found another"
fix = "Check expected type, convert with .into(), as, or From trait"

[[error]]
code = "E0599"
cause = "Method or associated item not found on type"
fix = "Check method name, required traits (use), and type signature"

[[error]]
code = "E0425"
cause = "Cannot find value in this scope"
fix = "Check spelling, imports, and variable scope"
```

- [ ] **Step 3: Create HintDb with tests**

Create `src/daemon/parser/hint.rs`:

```rust
//! Error code → fix suggestion lookup database.
//!
//! Loads error codes from TOML files at compile time via `include_str!`.
//! Provides a `HintDb` singleton for looking up hints by language + code.

use arshy_lib::ipc::EventHint;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
struct ErrorFile {
    meta: ErrorFileMeta,
    error: Vec<ErrorEntry>,
}

#[derive(Debug, Deserialize)]
struct ErrorFileMeta {
    language: String,
}

#[derive(Debug, Deserialize)]
struct ErrorEntry {
    code: String,
    cause: String,
    fix: Option<String>,
}

/// Singleton hint database loaded at startup.
static HINT_DB: LazyLock<HintDb> = LazyLock::new(HintDb::load);

pub struct HintDb {
    entries: HashMap<(String, String), EventHint>,
}

impl HintDb {
    fn load() -> Self {
        let mut entries = HashMap::new();
        let files: &[(&str, &str)] = &[
            ("rust", include_str!("../../../parsers/errors/rust.toml")),
            // Add more as they are created:
            // ("typescript", include_str!("../../../parsers/errors/typescript.toml")),
            // ("python", include_str!("../../../parsers/errors/python.toml")),
            // ("go", include_str!("../../../parsers/errors/go.toml")),
        ];

        for &(language, content) in files {
            match toml::from_str::<ErrorFile>(content) {
                Ok(file) => {
                    for entry in file.error {
                        entries.insert(
                            (language.to_string(), entry.code),
                            EventHint {
                                cause: entry.cause,
                                fix: entry.fix,
                            },
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

    /// Get the global hint database.
    pub fn get() -> &'static HintDb {
        &HINT_DB
    }

    /// Look up a hint for a given language and error code.
    pub fn lookup(&self, language: &str, code: &str) -> Option<&EventHint> {
        self.entries.get(&(language.to_string(), code.to_string()))
    }

    /// Number of entries in the database (for stats/testing).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the database is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Map tool name to language for hint lookup.
pub fn tool_to_language(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "cargo" | "cargo-test" | "clippy" | "rustc" => Some("rust"),
        "tsc" | "eslint" | "biome" | "oxlint" | "prettier" | "swc" | "esbuild"
        | "vite" | "webpack" | "vitest" | "jest" | "mocha" => Some("typescript"),
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
        };
        let json = serde_json::to_string(&hint).unwrap();
        let back: EventHint = serde_json::from_str(&json).unwrap();
        assert_eq!(hint, back);
    }
}
```

- [ ] **Step 4: Declare module in parser/mod.rs**

In `src/daemon/parser/mod.rs`, add:

```rust
pub mod hint;
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib --bin arshy --bin arshyd 2>&1 | tail -10`
Expected: All tests PASS (including new hint tests)

- [ ] **Step 6: Commit**

```bash
git add src/ipc/mod.rs src/daemon/parser/hint.rs src/daemon/parser/mod.rs parsers/errors/rust.toml
git commit -m "feat(parser): add EventHint struct and HintDb with Rust error codes

HintDb loads error codes from parsers/errors/*.toml at startup.
Currently covers 3 Rust codes (E0308, E0599, E0425) for testing.
More languages and codes added in subsequent tasks."
```

---

### Task 2: Error code databases (TypeScript, Python, Go)

**Files:**
- Create: `parsers/errors/typescript.toml`
- Create: `parsers/errors/python.toml`
- Create: `parsers/errors/go.toml`
- Modify: `src/daemon/parser/hint.rs` (uncomment include_str! lines)

- [ ] **Step 1: Create TypeScript error codes**

Create `parsers/errors/typescript.toml`:

```toml
[meta]
language = "typescript"

[[error]]
code = "TS2345"
cause = "Argument type not assignable to parameter type"
fix = "Check parameter type annotation, cast with 'as' or update argument"

[[error]]
code = "TS2339"
cause = "Property does not exist on type"
fix = "Check spelling, add optional chaining (?.), or extend the type"

[[error]]
code = "TS2322"
cause = "Type not assignable to target type"
fix = "Check the assignment target type and adjust the value"

[[error]]
code = "TS7006"
cause = "Parameter implicitly has 'any' type"
fix = "Add explicit type annotation to the parameter"

[[error]]
code = "TS2531"
cause = "Object is possibly null"
fix = "Add null check (if (x !== null)) or use non-null assertion (!)"

[[error]]
code = "TS2532"
cause = "Object is possibly undefined"
fix = "Add undefined check or use optional chaining (?.)"

[[error]]
code = "TS2307"
cause = "Cannot find module"
fix = "Check module name spelling, install with npm/yarn, check tsconfig paths"

[[error]]
code = "TS2554"
cause = "Expected N arguments but got M"
fix = "Check function signature and provide correct number of arguments"

[[error]]
code = "TS2464"
cause = "A computed property name must be of type string, number, symbol, or any"
fix = "Use a string/number/symbol literal or add type annotation"

[[error]]
code = "TS18048"
cause = "Value is possibly 'undefined'"
fix = "Add a guard: if (value !== undefined) { ... }"

[[error]]
code = "TS2769"
cause = "No overload matches this call"
fix = "Check function overloads and provide matching argument types"

[[error]]
code = "TS6133"
cause = "Declared but never read"
fix = "Remove unused variable or prefix with underscore (_var)"

[[error]]
code = "TS2571"
cause = "Object is of type 'unknown'"
fix = "Add type assertion (as Type) or type guard (if (typeof x === '...'))"

[[error]]
code = "TS2304"
cause = "Cannot find name"
fix = "Import the name or check spelling"

[[error]]
code = "TS2459"
cause = "Type has no property and no index signature"
fix = "Add the property to the type or use a type assertion"
```

- [ ] **Step 2: Create Python error codes**

Create `parsers/errors/python.toml`:

```toml
[meta]
language = "python"

[[error]]
code = "NameError"
cause = "Variable or function name is not defined"
fix = "Check spelling, import the name, or define it before use"

[[error]]
code = "TypeError"
cause = "Operation on inappropriate type"
fix = "Check variable types, add type conversion (int(), str(), etc.)"

[[error]]
code = "AttributeError"
cause = "Object has no such attribute or method"
fix = "Check attribute name spelling and object type"

[[error]]
code = "ImportError"
cause = "Cannot import name from module"
fix = "Check module name and exported names, install package if needed"

[[error]]
code = "ModuleNotFoundError"
cause = "Module not found"
fix = "Install with pip install <package>, check sys.path"

[[error]]
code = "KeyError"
cause = "Dictionary key not found"
fix = "Use .get(key, default) or check 'key in dict' first"

[[error]]
code = "IndexError"
cause = "List index out of range"
fix = "Check list length, use len() guard or try/except"

[[error]]
code = "ValueError"
cause = "Function received argument of right type but inappropriate value"
fix = "Validate input before passing to function"

[[error]]
code = "FileNotFoundError"
cause = "File or directory does not exist"
fix = "Check path, use os.path.exists() before opening"

[[error]]
code = "PermissionError"
cause = "Insufficient permissions for file operation"
fix = "Check file permissions, run with appropriate privileges"

[[error]]
code = "SyntaxError"
cause = "Invalid Python syntax"
fix = "Check for missing colons, brackets, quotes, or indentation"

[[error]]
code = "IndentationError"
cause = "Incorrect indentation"
fix = "Use consistent indentation (4 spaces recommended)"

[[error]]
code = "ZeroDivisionError"
cause = "Division or modulo by zero"
fix = "Add guard: if denominator != 0"

[[error]]
code = "RuntimeError"
cause = "Generic runtime error"
fix = "Read the full traceback for context-specific fix"
```

- [ ] **Step 3: Create Go error codes**

Create `parsers/errors/go.toml`:

```toml
[meta]
language = "go"

[[error]]
code = "undeclared"
cause = "Undeclared name or variable"
fix = "Declare the variable or import the package"

[[error]]
code = "cannot use"
cause = "Type mismatch in assignment or function call"
fix = "Convert type with T(x) or check function signature"

[[error]]
code = "missing return"
cause = "Function missing return statement at end"
fix = "Add return statement at the end of the function"

[[error]]
code = "not enough arguments"
cause = "Too few arguments in function call"
fix = "Check function signature and provide all required arguments"

[[error]]
code = "too many arguments"
cause = "Too many arguments in function call"
fix = "Remove extra arguments or check function signature"

[[error]]
code = "undefined"
cause = "Type or field not defined"
fix = "Check spelling, import, or struct field definition"

[[error]]
code = "imported and not used"
cause = "Package imported but not referenced"
fix = "Remove unused import or use the package"

[[error]]
code = "declared and not used"
cause = "Variable declared but never read"
fix = "Use the variable or assign to _ (blank identifier)"

[[error]]
code = "cannot assign"
cause = "Cannot assign to value (e.g., map not initialized)"
fix = "Initialize with make() before assignment: m = make(map[K]V)"

[[error]]
code = "nil pointer"
cause = "Nil pointer dereference"
fix = "Add nil check before accessing: if p != nil { p.Field }"

[[error]]
code = "index out of range"
cause = "Slice or array index out of bounds"
fix = "Check len(s) before accessing s[i]"
```

- [ ] **Step 4: Enable all languages in HintDb**

In `src/daemon/parser/hint.rs`, uncomment the include_str! lines:

```rust
let files: &[(&str, &str)] = &[
    ("rust", include_str!("../../../parsers/errors/rust.toml")),
    ("typescript", include_str!("../../../parsers/errors/typescript.toml")),
    ("python", include_str!("../../../parsers/errors/python.toml")),
    ("go", include_str!("../../../parsers/errors/go.toml")),
];
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib --bin arshyd hint -- --nocapture 2>&1 | tail -10`
Expected: All hint tests PASS, len() returns ~100

- [ ] **Step 6: Commit**

```bash
git add parsers/errors/ src/daemon/parser/hint.rs
git commit -m "feat(parser): add error code databases for Rust, TypeScript, Python, Go

~100 error codes with cause + fix suggestions across 4 languages."
```

---

### Task 3: Enrichment in executor

**Files:**
- Modify: `src/daemon/exec/mod.rs` (add hint enrichment in sync path)

- [ ] **Step 1: Add hint enrichment to sync completion path**

In `src/daemon/exec/mod.rs`, in the sync completion path where events are enriched (after ContextEnricher, before compute_summary), add hint enrichment:

```rust
// Enrich error events with fix hints
if let Some(ref mut evts) = events_json {
    let hint_db = super::parser::hint::HintDb::get();
    let language = t.detected_tool.as_ref()
        .and_then(|tool| super::parser::hint::tool_to_language(&tool.tool_name));
    if let Some(lang) = language {
        for evt in evts.iter_mut() {
            if evt.get("hint").is_some() {
                continue; // already has hint
            }
            if let Some(code) = evt.get("code").and_then(|v| v.as_str()) {
                if let Some(hint) = hint_db.lookup(lang, code) {
                    evt["hint"] = serde_json::to_value(hint).unwrap_or_default();
                }
            }
        }
    }
}
```

IMPORTANT: Read the actual sync completion path first. The enrichment should happen AFTER events are fetched from store and AFTER errors_only filter, but BEFORE compute_summary. Place it adjacent to the existing ContextEnricher enrichment.

- [ ] **Step 2: Run tests**

Run: `cargo test --lib --bin arshy --bin arshyd 2>&1 | tail -5`
Expected: All tests PASS

- [ ] **Step 3: Commit**

```bash
git add src/daemon/exec/mod.rs
git commit -m "feat(exec): enrich error events with fix hints from HintDb

Error events with a code field now get automatic hint attachment
based on the detected tool's language. Hints include cause + fix."
```

---

## Final Verification

- [ ] `cargo test --lib --bin arshy --bin arshyd` — all tests pass
- [ ] `cargo fmt --all -- --check` — clean
- [ ] `cargo clippy --all-targets -- -D warnings` — clean
- [ ] Manual: `arshy run "cargo build" --format json` on a project with type errors → verify `hint` field in error events
