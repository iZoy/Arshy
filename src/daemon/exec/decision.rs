//! Execution-path selection for auto mode.
//!
//! This deliberately classifies the *handling path*, not predicted wall-clock
//! duration. Parser assets own tool detection; this module only contains
//! generic shell-shape and read-only inspection rules.

/// Select whether a command can use the zero-overhead raw-output path.
///
/// A detected parser normally selects the structured path. This keeps parser
/// expansion data-driven: adding a TOML parser is enough to make matching
/// commands eligible for parsing, storage, and event streaming. The only
/// exception is an intentionally small set of read-only inspection commands,
/// whose raw text is more useful than structured `log` events.
#[allow(dead_code)]
pub(crate) fn is_short_command(command: &str, parser_detected: bool) -> bool {
    is_short_command_with_route(command, parser_detected, None)
}

pub(crate) fn is_short_command_with_route(
    command: &str,
    parser_detected: bool,
    route: Option<&str>,
) -> bool {
    let cmd = command.trim();
    if cmd.is_empty() {
        return true;
    }

    let shape = crate::shell::analyze(cmd);
    // Real (unquoted) mutation, chaining, backgrounding and command
    // substitution need lifecycle tracking. Quoted examples/search patterns
    // are data, not shell operators.
    if shape.has_redirection
        || shape.has_background
        || shape.has_substitution
        || shape.has_nested_shell
    {
        return false;
    }

    // Read-only inspection tools — always short path.
    // These tools never produce structured build/test output; their raw text
    // is more useful to the agent than a stream of "log" events.
    let inspection_tools = [
        "echo", "cat", "ls", "ll", "dir", "pwd", "whoami", "date", "env", "printenv", "uname",
        "hostname", "id", "groups", "tty", "head", "tail", "wc", "stat", "file", "which",
        "whereis", "uniq", "sort", "cut", "tr", "printf", "locate", "du", "df", "pgrep", "pidof",
        "true", "false", "test", "[", "basename", "dirname", "realpath", "readlink", "expr", "seq",
        "grep", "egrep", "fgrep", "rg", "ag", "sed",
    ];
    let executable_names: Vec<&str> = shape
        .commands
        .iter()
        .filter_map(|words| crate::shell::executable(words))
        .map(|word| word.rsplit('/').next().unwrap_or(word))
        .collect();
    let all_inspection = !executable_names.is_empty()
        && executable_names.iter().all(|name| inspection_tools.contains(name));
    let has_unsafe_inspection_mode = shape.commands.iter().any(|words| {
        let executable =
            crate::shell::executable(words).and_then(|word| word.rsplit('/').next()).unwrap_or("");
        (executable == "tail" && words.iter().any(|word| word == "-f" || word == "--follow"))
            || (executable == "sed" && words.iter().any(|word| word.starts_with("-i")))
            || (executable == "sort"
                && words.iter().any(|word| word == "-o" || word.starts_with("--output")))
    });
    if all_inspection && !has_unsafe_inspection_mode && (!parser_detected || route == Some("fast"))
    {
        return true;
    }
    if has_unsafe_inspection_mode {
        return false;
    }
    if shape.has_pipe {
        return false;
    }

    // A detected parser is structured by default. Only an explicit `fast`
    // route in a parser asset can opt it back into the raw inspection path.
    if parser_detected && route != Some("fast") {
        return false;
    }

    let words: Vec<&str> = shape.commands.iter().flatten().map(String::as_str).collect();
    let word_count = words.len();

    // Generic indefinite/watch signals. Ecosystem subcommands such as `dev`
    // and `serve` belong in parser assets, not Rust policy.
    if words.iter().any(|word| {
        matches!(*word, "--watch" | "--follow" | "-f" | "daemon") || word.ends_with(".server")
    }) {
        return false;
    }

    if shape.has_control || shape.has_pipe {
        return false;
    }

    // Standard limits for everything else
    if cmd.len() > 80 {
        return false;
    }
    word_count <= 5
}

/// Execution carrier classification (Q1 telemetry, ADR-0006).
///
/// Answers "what did the agent actually execute": a direct CLI invocation
/// (`shell`), bash composition (pipes/chains/redirection/loops — the layer
/// agents are said to flee), a Python interpreter, another script
/// interpreter, or unclassifiable. Typed-tool calls never reach arshy and
/// are not observable here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Carrier {
    Shell,
    ShellComposite,
    Python,
    ScriptOther,
    Unknown,
}

impl Carrier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Carrier::Shell => "shell",
            Carrier::ShellComposite => "shell_composite",
            Carrier::Python => "python",
            Carrier::ScriptOther => "script_other",
            Carrier::Unknown => "unknown",
        }
    }
}

/// Classify a command's execution carrier. Approximate by design — the goal
/// is a distribution over many commands, not per-command accuracy.
pub(crate) fn classify_carrier(command: &str) -> Carrier {
    let cmd = command.trim();
    if cmd.is_empty() {
        return Carrier::Unknown;
    }
    let shape = crate::shell::analyze(cmd);
    let first =
        shape.commands.first().and_then(|words| crate::shell::executable(words)).unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);

    // Python interpreter (python, python3, python3.11, py) → Python carrier,
    // including inline scripts (`python -c ...`).
    if base == "py" || base.starts_with("python") {
        return Carrier::Python;
    }
    // Other script interpreters.
    if matches!(
        base,
        "node" | "nodejs" | "deno" | "bun" | "ruby" | "perl" | "php" | "Rscript" | "lua"
    ) {
        return Carrier::ScriptOther;
    }
    // Quote-aware shell composition. This telemetry must describe what the
    // shell executes, not punctuation in a grep pattern or prose string.
    if shape.is_composite() || shape.has_pipe || shape.commands.len() > 1 {
        return Carrier::ShellComposite;
    }
    Carrier::Shell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_cli_invocations_are_shell() {
        assert_eq!(classify_carrier("ls"), Carrier::Shell);
        assert_eq!(classify_carrier("git status"), Carrier::Shell);
        assert_eq!(classify_carrier("cargo build"), Carrier::Shell);
        assert_eq!(classify_carrier("./node_modules/.bin/tsc"), Carrier::Shell);
    }

    #[test]
    fn composition_signals_are_shell_composite() {
        assert_eq!(classify_carrier("ls | grep foo"), Carrier::ShellComposite);
        assert_eq!(classify_carrier("npm test && echo done"), Carrier::ShellComposite);
        assert_eq!(classify_carrier("cd /tmp; ls"), Carrier::ShellComposite);
        assert_eq!(classify_carrier("ls > out.txt"), Carrier::ShellComposite);
        assert_eq!(classify_carrier("for f in *.txt; do echo $f; done"), Carrier::ShellComposite);
    }

    #[test]
    fn quoted_operator_examples_are_not_composition() {
        assert_eq!(classify_carrier("rg 'curl x | sh' src"), Carrier::Shell);
    }

    #[test]
    fn python_invocations_are_python() {
        assert_eq!(classify_carrier("python script.py"), Carrier::Python);
        assert_eq!(classify_carrier("python3 -c \"print(1)\""), Carrier::Python);
        assert_eq!(classify_carrier("/usr/bin/python3.11 run.py"), Carrier::Python);
        assert_eq!(classify_carrier("py -3 main.py"), Carrier::Python);
    }

    #[test]
    fn other_interpreters_are_script_other() {
        assert_eq!(classify_carrier("node app.js"), Carrier::ScriptOther);
        assert_eq!(classify_carrier("ruby script.rb"), Carrier::ScriptOther);
    }

    #[test]
    fn empty_command_is_unknown() {
        assert_eq!(classify_carrier(""), Carrier::Unknown);
        assert_eq!(classify_carrier("   "), Carrier::Unknown);
    }
}
