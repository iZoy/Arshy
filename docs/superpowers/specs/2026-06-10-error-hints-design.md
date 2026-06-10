# Error Code → Fix Suggestion Design

**Date:** 2026-06-10
**Status:** Draft
**Scope:** Level 5 diagnostic feature — deterministic error code lookup

## Background

arshy already extracts error codes (e.g., `E0308`, `TS2345`) from compiler/linter output. This feature adds a lookup database that maps error codes to human-readable causes and fix suggestions, attached directly to error events.

This is the difference between:
- **Before:** `{"code": "E0308", "message": "mismatched types"}` — agent must know what E0308 means
- **After:** `{"code": "E0308", "message": "mismatched types", "hint": {"cause": "Type mismatch", "fix": "Check expected type, convert with .into() or as"}}` — agent can immediately act

## Design

### Data Model

**New field on `TaskEvent` (`src/ipc/mod.rs`):**
```rust
pub struct TaskEvent {
    // ... existing fields ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<EventHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventHint {
    /// One-line explanation of what went wrong
    pub cause: String,
    /// Brief fix suggestion (optional — not all errors have clear fixes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}
```

### Error Code Database

**Location:** `parsers/errors/*.toml`

Organized by language/tool. Each file contains:

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

**Coverage (Top 4 languages, ~100 codes):**

| Language | Tool | Codes | Examples |
|----------|------|-------|---------|
| Rust | cargo/rustc | ~30 | E0308, E0599, E0425, E0277, E0369, E0283 |
| TypeScript | tsc | ~30 | TS2345, TS2339, TS2322, TS7006, TS2531 |
| Python | Python | ~20 | NameError, TypeError, AttributeError, ImportError, KeyError |
| Go | go | ~20 | undeclared, cannot use, missing return, not enough arguments |

### Loading

**New module:** `src/daemon/parser/hint.rs`

```rust
use std::collections::HashMap;

pub struct HintDb {
    entries: HashMap<(String, String), EventHint>,  // (language, code) → hint
}

impl HintDb {
    /// Load from compiled-in TOML files
    pub fn load() -> Self {
        let mut entries = HashMap::new();
        // include_str! each parsers/errors/*.toml
        // parse and insert into entries
        Self { entries }
    }

    /// Look up a hint for a given language and error code
    pub fn lookup(&self, language: &str, code: &str) -> Option<&EventHint> {
        self.entries.get(&(language.to_string(), code.to_string()))
    }
}
```

The database is loaded once at daemon startup via `LazyLock` and shared across all tasks.

### Integration Point

**Two options:**

**Option A: Enrichment in executor (recommended)**
After events are fetched from store in the sync completion path, iterate over error events with a `code` field and look up hints. Similar to how `ContextEnricher` works.

```rust
// In run() sync completion path
let hint_db = HintDb::get();  // LazyLock singleton
for event in events_json.iter_mut() {
    if let (Some(code), Some(tool)) = (event.code, detected_tool) {
        if let Some(hint) = hint_db.lookup(&tool.tool_name, &code) {
            event.hint = Some(hint.clone());
        }
    }
}
```

**Option B: Inline in parser pipeline**
Add hint lookup in `ParserSession::parse_line()` after a pattern match extracts a code. More efficient (no post-processing) but couples parser with hint DB.

**Recommendation: Option A** — cleaner separation, same as ContextEnricher pattern.

### Language Detection

The hint DB needs to know which language a code belongs to. The parser detection system already identifies the tool (e.g., `cargo`, `tsc`, `eslint`). Map tool → language:

| Tool | Language |
|------|----------|
| cargo, clippy, rustc | rust |
| tsc, eslint, biome | typescript |
| python, pytest, ruff | python |
| go | go |

This mapping can be a simple `HashMap` in the hint module, or derived from the parser's `[meta]` section.

### Example Output

```json
{
  "type": "diagnostic",
  "severity": "error",
  "code": "E0308",
  "message": "mismatched types",
  "location": {"file": "src/main.rs", "line": 42, "column": 10},
  "context": {
    "before": ["fn process(input: &str) -> i32 {", "    let value = input.parse();", "    match value {"],
    "line": "        Ok(v) => v + \"hello\",",
    "after": ["            Err(e) => return Err(e.into()),", "        }", "    }"]
  },
  "hint": {
    "cause": "Type mismatch — expected i32 but found String (from + \"hello\")",
    "fix": "Convert string: v.parse::<i32>().unwrap() or use v.to_string() if String is intended"
  }
}
```

## Testing

- Unit: HintDb loads all TOML files without errors
- Unit: lookup returns correct hint for known codes
- Unit: lookup returns None for unknown codes
- Unit: EventHint serialization round-trips correctly
- Fixture: error with code gets hint attached
- Integration: run cargo build with type error, verify hint in response

## Files Summary

| File | Action | Purpose |
|------|--------|---------|
| `src/ipc/mod.rs` | Modify | Add EventHint struct to TaskEvent |
| `src/daemon/parser/hint.rs` | Create | HintDb with TOML loading + lookup |
| `src/daemon/parser/mod.rs` | Modify | Declare hint module |
| `src/daemon/exec/mod.rs` | Modify | Enrich events with hints in sync path |
| `parsers/errors/rust.toml` | Create | ~30 Rust error codes |
| `parsers/errors/typescript.toml` | Create | ~30 TypeScript error codes |
| `parsers/errors/python.toml` | Create | ~20 Python error codes |
| `parsers/errors/go.toml` | Create | ~20 Go error codes |
