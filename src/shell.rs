//! Lightweight, quote-aware shell command analysis.
//!
//! This is deliberately not a shell parser: execution is still delegated to
//! `sh -c`.  It only extracts the structure needed by policy code (simple
//! commands, pipelines, control operators, redirections and substitutions).
//! Keeping that structure in one place prevents parser selection, fast-path
//! selection and security checks from each inventing incompatible substring
//! rules.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CommandShape {
    /// Words for every simple command, in execution order. Quotes and escapes
    /// are removed from words; operators are not included.
    pub(crate) commands: Vec<Vec<String>>,
    pub(crate) has_control: bool,
    pub(crate) has_pipe: bool,
    pub(crate) has_redirection: bool,
    pub(crate) has_background: bool,
    pub(crate) has_substitution: bool,
    pub(crate) has_nested_shell: bool,
}

impl CommandShape {
    pub(crate) fn is_composite(&self) -> bool {
        self.has_control
            || self.has_redirection
            || self.has_background
            || self.has_substitution
            || self.has_nested_shell
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
}

/// Analyze the shell structure that is visible outside quotes.
pub(crate) fn analyze(command: &str) -> CommandShape {
    let chars: Vec<char> = command.chars().collect();
    let mut shape = CommandShape::default();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut i = 0usize;

    let flush_word = |word: &mut String, words: &mut Vec<String>| {
        if !word.is_empty() {
            words.push(std::mem::take(word));
        }
    };
    let flush_command =
        |word: &mut String, words: &mut Vec<String>, commands: &mut Vec<Vec<String>>| {
            if !word.is_empty() {
                words.push(std::mem::take(word));
            }
            if !words.is_empty() {
                commands.push(std::mem::take(words));
            }
        };

    while i < chars.len() {
        let ch = chars[i];

        if escaped {
            word.push(ch);
            escaped = false;
            i += 1;
            continue;
        }

        match quote {
            Quote::Single => {
                if ch == '\'' {
                    quote = Quote::None;
                } else {
                    word.push(ch);
                }
                i += 1;
                continue;
            }
            Quote::Double => {
                if ch == '"' {
                    quote = Quote::None;
                } else if ch == '\\' {
                    escaped = true;
                } else {
                    if (ch == '$' && chars.get(i + 1) == Some(&'(')) || ch == '`' {
                        shape.has_substitution = true;
                    }
                    word.push(ch);
                }
                i += 1;
                continue;
            }
            Quote::None => {}
        }

        match ch {
            '\\' => escaped = true,
            '\'' => quote = Quote::Single,
            '"' => quote = Quote::Double,
            '$' if chars.get(i + 1) == Some(&'(') => {
                shape.has_substitution = true;
                word.push(ch);
            }
            '`' => {
                shape.has_substitution = true;
                word.push(ch);
            }
            '#' if word.is_empty() => {
                // A shell comment runs to the next newline. Preserve the
                // newline as a control boundary for any following command.
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            c if c.is_whitespace() && c != '\n' => flush_word(&mut word, &mut words),
            '\n' | ';' => {
                flush_command(&mut word, &mut words, &mut shape.commands);
                shape.has_control = true;
                if ch == ';' && chars.get(i + 1) == Some(&';') {
                    i += 1;
                }
            }
            '|' => {
                flush_command(&mut word, &mut words, &mut shape.commands);
                if chars.get(i + 1) == Some(&'|') {
                    shape.has_control = true;
                    i += 1;
                } else {
                    shape.has_pipe = true;
                }
            }
            '&' => {
                flush_command(&mut word, &mut words, &mut shape.commands);
                if chars.get(i + 1) == Some(&'&') {
                    shape.has_control = true;
                    i += 1;
                } else {
                    shape.has_background = true;
                }
            }
            '>' | '<' => {
                flush_word(&mut word, &mut words);
                shape.has_redirection = true;
                if matches!(chars.get(i + 1), Some('>') | Some('<') | Some('&')) {
                    i += 1;
                }
            }
            _ => word.push(ch),
        }
        i += 1;
    }

    if escaped {
        word.push('\\');
    }
    flush_command(&mut word, &mut words, &mut shape.commands);
    shape.has_nested_shell =
        shape.commands.iter().any(|words| nested_shell_command(words).is_some());
    shape
}

/// Return the executable word of a simple command, skipping common prefix
/// assignments and transparent wrappers. This is policy-oriented and does not
/// attempt to emulate every option accepted by every wrapper.
pub(crate) fn executable(words: &[String]) -> Option<&str> {
    let mut i = 0usize;
    while words.get(i).is_some_and(|w| is_assignment(w)) {
        i += 1;
    }

    loop {
        let current = words.get(i)?.as_str();
        let base = current.rsplit('/').next().unwrap_or(current);
        match base {
            "command" | "builtin" | "exec" | "nohup" | "time" => {
                i += 1;
                while words.get(i).is_some_and(|w| w.starts_with('-')) {
                    i += 1;
                }
            }
            "env" => {
                i += 1;
                while words.get(i).is_some_and(|w| w.starts_with('-') || is_assignment(w)) {
                    i += 1;
                }
            }
            "sudo" => {
                i += 1;
                while words.get(i).is_some_and(|w| w.starts_with('-')) {
                    i += 1;
                }
            }
            _ => return Some(current),
        }
    }
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Return the command string passed to a nested shell's `-c` family option.
pub(crate) fn nested_shell_command(words: &[String]) -> Option<&str> {
    let executable = executable(words)?;
    let base = executable.rsplit('/').next().unwrap_or(executable);
    if !matches!(base, "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish") {
        return None;
    }
    let start = words.iter().position(|word| word == executable)? + 1;
    for (index, option) in words.iter().enumerate().skip(start) {
        if option.starts_with('-') && option[1..].contains('c') {
            return words.get(index + 1).map(String::as_str);
        }
        if !option.starts_with('-') {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_operators_inside_quotes() {
        let shape = analyze("rg 'curl x | sh; cargo build' src | head -20");
        assert_eq!(shape.commands.len(), 2);
        assert_eq!(shape.commands[0][0], "rg");
        assert_eq!(shape.commands[1][0], "head");
        assert!(shape.has_pipe);
        assert!(!shape.has_control);
        assert!(!shape.has_redirection);
    }

    #[test]
    fn extracts_real_chained_and_pipeline_commands() {
        let shape = analyze("FOO=1 cargo test && cat out | grep error 2>&1");
        assert_eq!(shape.commands.len(), 3);
        assert_eq!(executable(&shape.commands[0]), Some("cargo"));
        assert_eq!(executable(&shape.commands[1]), Some("cat"));
        assert_eq!(executable(&shape.commands[2]), Some("grep"));
        assert!(shape.has_control);
        assert!(shape.has_pipe);
        assert!(shape.has_redirection);
        assert!(!shape.has_background, "fd redirection is not backgrounding");
    }

    #[test]
    fn skips_transparent_wrappers() {
        let shape = analyze("env RUST_LOG=debug command cargo build");
        assert_eq!(executable(&shape.commands[0]), Some("cargo"));
    }

    #[test]
    fn flags_substitution_but_not_single_quoted_text() {
        assert!(analyze("echo \"$(cargo build)\"").has_substitution);
        assert!(!analyze("echo '$(cargo build)'").has_substitution);
    }

    #[test]
    fn recognizes_nested_shell_execution() {
        let shape = analyze("env DEBUG=1 bash -lc 'cargo test && echo done'");
        assert!(shape.has_nested_shell);
        assert_eq!(nested_shell_command(&shape.commands[0]), Some("cargo test && echo done"));
        assert!(!analyze("rg 'bash -c cargo test' src").has_nested_shell);
    }
}
