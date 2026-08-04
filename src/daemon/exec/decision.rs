//! Short-command detection — decides eligibility for the zero-overhead sync path.

/// Determine whether a command is "short" — eligible for zero-overhead sync path.
///
/// Short commands skip store insertion, parser session, and event streaming.
/// They return raw stdout directly, matching the experience of a native shell tool.
pub(crate) fn is_short_command(command: &str) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() {
        return true;
    }
    // Pipes, redirects, chaining, backgrounding → non-short
    // Note: '|' is intentionally allowed — simple pipes (≤5 words, ≤80 chars)
    // take the fast short path; multi-pipe chains are caught by word-count limit.
    if cmd.contains(">>") || cmd.contains("&&") || cmd.contains("||") || cmd.contains('&') {
        return false;
    }
    // Long-running flags → non-short
    let long_flags = ["--watch", "-f", "serve", "daemon", "start", "dev", "preview"];
    if long_flags.iter().any(|f| cmd.contains(f)) {
        return false;
    }

    // Split whitespace once and reuse
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let word_count = words.len();
    let first_word = words.first().copied().unwrap_or("");
    let first_two = if words.len() >= 2 {
        // Avoid allocation: just check starts_with on the original command
        // after the first word. But since we need first_two for matching,
        // build it from the words we already have.
        &cmd[..cmd.len().min(first_word.len() + 1 + words.get(1).map(|w| w.len()).unwrap_or(0))]
    } else {
        first_word
    };

    // Path-style invocations (./node_modules/.bin/tsc, /usr/bin/tsc, ...) must
    // still match by basename — otherwise every bin-path call falls through to
    // the short path and the parser never runs.
    let first_word_base = first_word.rsplit('/').next().unwrap_or(first_word);
    // Three-word prefix for multi-word tools (python -m pytest, ...) — the
    // two-word slice below cannot see the third word and would miss them.
    let first_three = if words.len() >= 3 {
        &cmd[..cmd.len().min(first_word.len() + 1 + words[1].len() + 1 + words[2].len())]
    } else {
        first_two
    };

    // Build/test/install commands always produce substantial output → non-short
    let long_output_prefixes = [
        "cargo test",
        "cargo build",
        "cargo clippy",
        "cargo bench",
        "cargo doc",
        "cargo run",
        "cargo fmt",
        "rustc",
        "npm test",
        "npm run",
        "npm install",
        "npm ci",
        "npx",
        "yarn test",
        "yarn run",
        "yarn install",
        "pnpm test",
        "pnpm run",
        "pnpm install",
        "pytest",
        "python -m pytest",
        "python3 -m pytest",
        "tsc",
        "eslint",
        "oxlint",
        "biome",
        "ruff",
        "uv",
        "go test",
        "go build",
        "go run",
        "go vet",
        "go lint",
        "make",
        "make test",
        "make build",
        "gradle",
        "./gradlew",
        "mvn",
        "pip install",
        "pip3 install",
        "docker build",
        "docker compose",
        "cmake",
        "ninja",
        "gcc",
        "clang",
        "g++",
        "clang++",
    ];
    if long_output_prefixes.iter().any(|p| first_three.starts_with(p) || first_word_base == *p) {
        return false;
    }

    // Read-only inspection tools — always short path.
    // These tools never produce structured build/test output; their raw text
    // is more useful to the agent than a stream of "log" events.
    let inspection_tools = [
        "echo",
        "cat",
        "ls",
        "ll",
        "dir",
        "pwd",
        "whoami",
        "date",
        "env",
        "printenv",
        "uname",
        "hostname",
        "id",
        "groups",
        "tty",
        "head",
        "tail",
        "wc",
        "stat",
        "file",
        "which",
        "whereis",
        "sort",
        "uniq",
        "cut",
        "tr",
        "printf",
        "find",
        "locate",
        "du",
        "df",
        "pgrep",
        "pidof",
        "true",
        "false",
        "test",
        "[",
        "basename",
        "dirname",
        "realpath",
        "readlink",
        "expr",
        "seq",
        "tee",
        "grep",
        "egrep",
        "fgrep",
        "rg",
        "ag",
        "awk",
        "sed",
        "xargs",
        "git status",
        "git log",
        "git diff",
        "git branch",
        "git tag",
        "git show",
        "git stash",
        "git remote",
        "git config",
        "git", // covers "git -C <path> ..." and other git variants
    ];
    if inspection_tools.iter().any(|t| first_two.starts_with(t) || first_word == *t) {
        return true;
    }

    // Standard limits for everything else
    if cmd.len() > 80 {
        return false;
    }
    word_count <= 5
}
