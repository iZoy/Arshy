# Arshy Parser Specification

**Version: 1.0**

A parser definition describes how to extract structured events from a CLI tool's text output. This specification is independent of any specific implementation — any structured-shell runtime can adopt it.

---

## 1. Overview

CLI tools produce text. AI agents need structure. A parser definition bridges the gap: it declares regex patterns that match a tool's output lines and map them to typed events.

```
CLI output (text lines)
        |
        v
  Parser definition (this spec)
        |
        v
  Structured events (JSON)
```

A parser definition is a single TOML file. It contains:

1. **`[meta]`** — tool name, detection rules, parser type
2. **`[[pattern]]`** — one or more regex patterns that extract events from output lines

### Two parser tiers

| Tier | Format | Use case |
|------|--------|----------|
| **TOML (stateless)** | Regex line-by-line | Most tools — compiler errors, linter warnings, build output |
| **TOML (stateful)** | Regex + state machine | Tools whose output spans multiple lines and requires context |
| **Rhai script** | `.rhai` scripting | Complex multi-line output, deeply nested formats |

Tier selection is declared in `[meta] parser_type`. Most parsers use the default (stateless TOML).

---

## 2. Event Format

Every parser maps output lines to **events**. Events are JSON objects with a fixed schema:

```json
{
  "seq": 1,
  "type": "diagnostic",
  "severity": "error",
  "code": "TS2322",
  "message": "Type 'string' is not assignable to type 'number'",
  "location": {
    "file": "src/app.ts",
    "line": 10,
    "column": 5
  },
  "context": {
    "before": ["function greet() {", "  const x: number ="],
    "line": "  const x: number = \"hello\"",
    "after": ["  return x", "}", ""]
  }
}
```

### Event fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `seq` | integer | yes | Sequence number within the task (0-indexed) |
| `type` | string | yes | Event type (see below) |
| `severity` | string | no | `"error"`, `"warning"`, or `"info"` |
| `code` | string | no | Error/warning code (e.g. `"TS2322"`, `"E0308"`) |
| `message` | string | yes | Human-readable message |
| `location` | object | no | File location (see below) |
| `context` | object | no | Source lines around the location |

### Event types

| Type | Description |
|------|-------------|
| `diagnostic` | Error, warning, or info from a tool (compiler, linter, etc.) |
| `location` | A file:line:column reference without a standalone message |
| `summary` | End-of-task summary (error count, pass/fail status) |
| `crash` | Unhandled exception, panic, or traceback |
| `test_result` | Individual test pass/fail |
| `log` | General informational output |

### Location object

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `file` | string | yes | File path (absolute or relative to project root) |
| `line` | integer | yes | Line number (1-indexed) |
| `column` | integer | no | Column number (1-indexed) |

### Context object

| Field | Type | Description |
|-------|------|-------------|
| `before` | string[] | Up to 3 lines before the error line |
| `line` | string | The error line itself |
| `after` | string[] | Up to 3 lines after the error line |

Context is populated by the runtime by reading the source file — parsers do not produce it.

---

## 3. TOML Parser Definition

### 3.1 File structure

```toml
[meta]
name = "tsc"
description = "TypeScript compiler"
detect = ["tsc"]
priority = 50

[[pattern]]
name = "ts-error"
regex = '^(.+?)\((\d+),(\d+)\): error (TS\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }
```

### 3.2 `[meta]` section

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | yes | — | Unique identifier for this parser |
| `description` | string | no | `""` | Human-readable description |
| `detect` | string[] | yes | — | Command prefixes that trigger this parser |
| `detect_full` | string[] | no | `[]` | Multi-word command patterns (e.g. `["cargo test"]`) |
| `parser_type` | string | no | `"toml"` | `"toml"` (stateless) or `"stateful"` |
| `priority` | integer | no | `50` | Higher = checked first when multiple parsers match |
| `min_version` | string | no | — | Minimum tool version this parser supports |
| `max_version` | string | no | — | Maximum tool version this parser supports |
| `schema_version` | string | no | `"1.0"` | Spec version this parser was written against |
| `since_version` | string | no | — | Version of the parser registry when this was added |

#### Detection rules

`detect` values are matched against the **first word** of the command as substring match (case-insensitive).

```
detect = ["cargo"]    matches  "cargo build", "cargo test", "cargo-clippy"
detect = ["docker"]   matches  "docker build", "docker-compose up"
detect = ["go"]       matches  "go test", "go build"
```

`detect_full` values are matched against the **first two words** of the command.

```
detect_full = ["cargo test"]   matches  "cargo test --release" but NOT "cargo build"
```

#### Priority

When a command matches multiple parsers, the one with the highest `priority` wins. Default is `50`. Use `55` or higher to override a builtin parser.

### 3.3 `[[pattern]]` section

Each `[[pattern]]` defines one regex pattern that matches a single output line.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Unique identifier within this parser |
| `regex` | string | yes | Rust regex pattern (RE2 syntax, no backreferences) |
| `event_type` | string | yes | Event type to emit (see Event types above) |
| `severity` | string | yes | Default severity for this pattern |
| `fields` | map | no | Maps field names to regex capture group indices |
| `deprecated` | bool | no | If `true`, this pattern emits a runtime warning |
| `replaced_by` | string | no | Name of the replacement pattern |
| `since_version` | string | no | Parser version when this pattern was introduced |

#### Field mappings

`fields` maps semantic field names to regex capture group indices (1-indexed):

| Key | Maps to | Type |
|-----|---------|------|
| `file` | `location.file` | string |
| `line` | `location.line` | integer |
| `column` | `location.column` | integer |
| `code` | `code` | string |
| `message` | `message` | string |
| `severity` | `severity` (overrides pattern default) | string |

Example:

```toml
regex = '^(.+?):(\d+):(\d+): error (\w+): (.+)$'
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }
```

Capture group 0 (the entire match) is never used. If a field is not mapped, it is omitted from the event.

#### Regex rules

- Uses Rust `regex` crate syntax (RE2-compatible)
- No backreferences (`\1`, `\2`) — these are not supported
- No look-ahead/look-behind
- Patterns are checked against individual lines (not multi-line)
- ReDoS protection: patterns with catastrophic backtracking potential are rejected

### 3.4 Stateful patterns

When `parser_type = "stateful"`, patterns gain two additional fields:

| Field | Type | Description |
|-------|------|-------------|
| `state_condition` | string | Only match when state key has this value (`"key=value"`) |
| `state_transition` | string | Set state key to this value after matching (`"key=value"`) |

State is a simple key-value map scoped to a single command execution. All keys and values are strings.

```toml
[meta]
name = "npm"
detect = ["npm"]
parser_type = "stateful"

[[pattern]]
name = "npm-error"
regex = '^npm ERR! (.+)'
event_type = "diagnostic"
severity = "error"
fields = { message = 1 }
state_transition = "has_error=true"

[[pattern]]
name = "npm-summary"
regex = '^(\d+) packages?'
event_type = "summary"
severity = "info"
state_condition = "has_error=true"
```

In this example:
1. When `npm ERR!` is seen, an error event is emitted AND `has_error` is set to `"true"`
2. The summary pattern only matches if `has_error` is `"true"` — so a summary is only emitted when there was an error

---

## 4. Rhai Script Parser

For output formats that cannot be expressed as line-by-line regex (deeply nested, multi-line blocks, context-dependent), a `.rhai` script provides full control.

### 4.1 Script API

```rhai
fn on_line(line, ctx) {
    // Called once per output line
    // line: string — the current line
    // ctx  — context object (see below)
}

fn on_complete(exit_code, ctx) {
    // Called after the command finishes
    // exit_code: integer — process exit code
}
```

### 4.2 Context object (`ctx`)

| Method | Signature | Description |
|--------|-----------|-------------|
| `emit` | `ctx.emit(type, severity, message)` | Emit an event |
| `emit_at` | `ctx.emit_at(type, severity, message, file, line)` | Emit an event with location |
| `state_get` | `ctx.state_get(key) -> string` | Read state value |
| `state_set` | `ctx.state_set(key, value)` | Write state value |
| `line_number` | `ctx.line_number() -> int` | Current line number (1-indexed) |

### 4.3 Example

```rhai
fn on_line(line, ctx) {
    if line.starts_with("Error:") {
        ctx.emit("diagnostic", "error", line.subslicing(6));
    } else if line.starts_with("  at ") {
        // Multi-line pattern: error on previous line, location on this line
        ctx.emit_at("diagnostic", "error", ctx.state_get("last_error"), line.subslicing(5), 0);
    }
    ctx.state_set("last_error", line);
}

fn on_complete(exit_code, ctx) {
    if exit_code != 0 {
        ctx.emit("summary", "error", "Command failed with exit code " + exit_code.to_string());
    }
}
```

---

## 5. Built-in Crash Detection

Runtimes should include a fallback crash parser that detects unhandled exceptions across languages without requiring a parser definition. Recommended patterns:

| Language | Pattern | Example |
|----------|---------|---------|
| Python | `File "…", line N` | `File "app.py", line 42` |
| Python | `XxxError: message` | `ValueError: invalid literal` |
| Rust | `panicked at 'message', file:line:col` | `panicked at 'index out of bounds', src/main.rs:10:5` |
| Go | `file:line +0x…` | `main.go:42 +0x1234` |
| Node.js | `at function (file:line:col)` | `at Object.<anonymous> (app.js:10:5)` |
| Shell | `file: line N:` | `script.sh: line 5:` |

Crash events use `type = "crash"` and `code` set to the detected language.

---

## 6. Fixture Test Format

Each parser should have fixture tests to validate accuracy. A fixture consists of two files:

### 6.1 Input file (`.txt`)

Raw output lines from the tool, exactly as they appear in a terminal. One file per test case.

```
src/app.ts(10,5): error TS2322: Type 'string' is not assignable to type 'number'.
src/app.ts(15,3): warning TS6133: 'x' is declared but never used.
src/utils.ts(22,10): error TS2339: Property 'foo' does not exist on type 'Bar'.
Found 3 errors.
```

### 6.2 Expected output file (`.json`)

An array of expected events. Fields not present in an expected event are treated as wildcards (not asserted).

```json
[
  {"type": "diagnostic", "severity": "error", "code": "TS2322", "file": "src/app.ts", "line": 10, "column": 5},
  {"type": "diagnostic", "severity": "warning", "code": "TS6133", "file": "src/app.ts", "line": 15, "column": 3},
  {"type": "diagnostic", "severity": "error", "code": "TS2339", "file": "src/utils.ts", "line": 22, "column": 10},
  {"type": "summary", "severity": "error"}
]
```

### 6.3 Accuracy threshold

A parser passes if its field-level match rate is >= 95%. The match rate is calculated as:

```
matched_fields / total_fields_in_expected_events
```

Each field in each expected event is checked independently. Missing or incorrect fields count as mismatches.

### 6.4 Directory layout

```
parsers/
  builtin/
    tsc.toml                          # parser definition
    tests/
      tsc/
        basic-error.txt               # test input
        basic-error.json              # expected output
        warning-only.txt              # another test case
        warning-only.json
```

Test function naming convention: `fixture_<parser_name>_<test_case>`.

---

## 7. Complete Examples

### 7.1 Simple stateless parser (tsc)

```toml
[meta]
name = "tsc"
description = "TypeScript compiler"
detect = ["tsc"]
priority = 50

[[pattern]]
name = "ts-error"
regex = '^(.+?)\((\d+),(\d+)\): error (TS\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }

[[pattern]]
name = "ts-warning"
regex = '^(.+?)\((\d+),(\d+)\): warning (TS\d+): (.+)$'
event_type = "diagnostic"
severity = "warning"
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }

[[pattern]]
name = "ts-error-alt"
regex = '^(.+?):(\d+):(\d+) - error (TS\d+): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { file = 1, line = 2, column = 3, code = 4, message = 5 }

[[pattern]]
name = "ts-summary"
regex = '^Found (\d+) errors?\.$'
event_type = "summary"
severity = "error"
```

### 7.2 Multi-location parser (cargo / rustc)

```toml
[meta]
name = "cargo"
description = "Cargo build tool / rustc compiler"
detect = ["cargo", "rustc"]
priority = 50

[[pattern]]
name = "cargo-error"
regex = '^error\[([E]\d+)\]: (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { code = 1, message = 2 }

[[pattern]]
name = "cargo-location"
regex = '^\s*--> (.+?):(\d+):(\d+)$'
event_type = "location"
severity = "info"
fields = { file = 1, line = 2, column = 3 }

[[pattern]]
name = "cargo-warning"
regex = '^warning: (.+)$'
event_type = "diagnostic"
severity = "warning"
fields = { message = 1 }
```

Note: `cargo-location` uses `event_type = "location"` because the file:line:col appears on a separate line from the error message. The runtime correlates adjacent diagnostic + location events.

### 7.3 Stateful parser (npm)

```toml
[meta]
name = "npm"
description = "npm package manager"
detect = ["npm"]
parser_type = "stateful"
priority = 50

[[pattern]]
name = "npm-packages-added"
regex = '^added (\d+) packages?'
event_type = "summary"
severity = "info"
state_transition = "packages_added=done"

[[pattern]]
name = "npm-error"
regex = '^npm ERR! (.+)'
event_type = "diagnostic"
severity = "error"
fields = { message = 1 }
state_transition = "has_error=true"

[[pattern]]
name = "npm-warn"
regex = '^npm WARN (.+)'
event_type = "diagnostic"
severity = "warning"
fields = { message = 1 }

[[pattern]]
name = "npm-up-to-date"
regex = '^up to date,? audited (\d+) packages?'
event_type = "summary"
severity = "info"

[[pattern]]
name = "npm-audited"
regex = '^audited (\d+) packages?'
event_type = "summary"
severity = "info"
```

### 7.4 Infrastructure tool (terraform)

```toml
[meta]
name = "terraform"
description = "Terraform infrastructure tool"
detect = ["terraform"]
priority = 50

[[pattern]]
name = "terraform-error"
regex = '^Error: (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { message = 1 }

[[pattern]]
name = "terraform-warning"
regex = '^Warning: (.+)$'
event_type = "diagnostic"
severity = "warning"
fields = { message = 1 }

[[pattern]]
name = "terraform-location"
regex = '^\s+on (.+?) line (\d+):'
event_type = "location"
severity = "info"
fields = { file = 1, line = 2 }

[[pattern]]
name = "terraform-plan-summary"
regex = '^(Plan:|No changes\.|Apply complete!)'
event_type = "summary"
severity = "info"
```

---

## 8. Contributing a Parser

### Step-by-step

1. **Create the parser file** — `parsers/builtin/<tool>.toml`
2. **Create fixture test directory** — `parsers/builtin/tests/<tool>/`
3. **Add test input** — paste real tool output into `basic-error.txt`
4. **Run the test harness** — it auto-generates `basic-error.json` from your patterns
5. **Verify accuracy** — ensure field-level match rate >= 95%
6. **Submit a PR**

### Minimum viable parser

Three fields are all you need:

```toml
[meta]
name = "mycli"
detect = ["mycli"]

[[pattern]]
name = "catch-all"
regex = '(?i)(error|fail)'
event_type = "diagnostic"
severity = "error"
```

### Checklist

- [ ] `name` is unique (check existing parsers)
- [ ] `detect` matches the tool's command name
- [ ] At least one pattern captures errors
- [ ] At least one fixture test exists
- [ ] Field-level accuracy >= 95%
- [ ] No ReDoS-vulnerable regex patterns
- [ ] If replacing an existing parser, mark old patterns with `deprecated = true` and `replaced_by`

### Common patterns

**File:line:column error format** (gcc, eslint, tsc):
```toml
regex = '^(.+?):(\d+):(\d+): (error|warning): (.+)$'
fields = { file = 1, line = 2, column = 3, severity = 4, message = 5 }
```

**File(line,col) format** (MSVC, tsc alternate):
```toml
regex = '^(.+?)\((\d+),(\d+)\): (error|warning) \w+: (.+)$'
fields = { file = 1, line = 2, column = 3, severity = 4, message = 5 }
```

**Simple error/warning prefix** (docker, terraform, kubectl):
```toml
regex = '^(Error|error): (.+)$'
event_type = "diagnostic"
severity = "error"
fields = { message = 2 }
```

**Indented location line** (cargo, clippy, go):
```toml
regex = '^\s*--> (.+?):(\d+):(\d+)$'
event_type = "location"
severity = "info"
fields = { file = 1, line = 2, column = 3 }
```

---

## 9. Runtime Behavior

This section describes expected runtime behavior for implementations of this specification.

### 9.1 Parser selection

1. Extract the first word (and first two words) from the command
2. Match against all loaded parsers' `detect` and `detect_full` arrays
3. Among matching parsers, select the one with the highest `priority`
4. If `parse_hint` is provided and matches a parser name, use that parser regardless of detection

### 9.2 Pipeline order

Lines flow through parsers in this order:

1. **JSON/NDJSON detection** — if output is valid JSON, parse directly
2. **Stateful parsers** (TOML stateful / Rhai scripts)
3. **Stateless TOML parsers** — line-by-line regex matching
4. **Crash parser** — universal fallback for panics/tracebacks
5. **Raw fallback** — return stdout as plain text

### 9.3 Short command bypass

Commands identified as "short" (simple inspection commands like `ls`, `grep`, `git status`) skip the parser pipeline entirely and return raw text. This is an implementation detail, not part of the parser format.

### 9.4 Hot reload

Runtimes should support filesystem watching of parser directories. When a `.toml` file changes, the runtime reloads all parsers and logs a diff audit. This enables editing parsers without restarting the daemon.

---

## 10. Versioning

### Specification version

This spec follows semantic versioning. The current version is **1.0**.

- **Major** (2.0): Breaking changes to the TOML schema or event format
- **Minor** (1.1): New optional fields, new event types, new pattern features
- **Patch** (1.0.1): Clarifications, typo fixes, example updates

### Parser schema version

Each parser declares `schema_version` in `[meta]`. This tracks which spec version the parser was written against. Runtimes may use this to apply compatibility transformations.

### Pattern lifecycle

- New patterns should include `since_version`
- Deprecated patterns must set `deprecated = true` and `replaced_by`
- Never remove a pattern without a deprecation cycle (one minor version)
