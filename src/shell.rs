//! Lightweight, quote-aware shell command analysis.
//!
//! This is deliberately not a shell parser: execution is still delegated to
//! `sh -c`.  It only extracts the structure needed by policy code (simple
//! commands, pipelines, control operators, redirections, substitutions, and
//! escaped line continuations).
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
    AnsiC,
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
            // POSIX shells remove backslash-newline pairs before tokenizing;
            // preserving the newline here would make `r\\\nm` look unlike
            // the executable `rm` to policy checks.
            if ch != '\n' {
                word.push(ch);
            }
            escaped = false;
            i += 1;
            continue;
        }

        if quote == Quote::AnsiC {
            match ch {
                '\'' => quote = Quote::None,
                '\\' => {
                    if let Some(decoded) = decode_ansi_c_escape(&chars, &mut i) {
                        word.push(decoded);
                    } else {
                        word.push('\\');
                    }
                }
                _ => word.push(ch),
            }
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
            Quote::AnsiC => unreachable!("ANSI-C quotes are handled before this match"),
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
            '$' if chars.get(i + 1) == Some(&'\'') => {
                // Bash ANSI-C quoting is a literal word form. Decode it so
                // policy sees the executable/arguments Bash will receive.
                quote = Quote::AnsiC;
                i += 1;
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

/// Decode one escape in a Bash `$'...'` word. `index` points at the
/// backslash and is advanced through all characters consumed by the escape.
fn decode_ansi_c_escape(chars: &[char], index: &mut usize) -> Option<char> {
    let next_index = *index + 1;
    let escaped = *chars.get(next_index)?;
    let decoded = match escaped {
        'a' => '\u{7}',
        'b' => '\u{8}',
        'e' | 'E' => '\u{1b}',
        'f' => '\u{c}',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\u{b}',
        '\n' => {
            *index = next_index;
            return None;
        }
        '\\' | '\'' | '"' | '?' => escaped,
        'x' => {
            let mut value = 0u32;
            let mut end = next_index + 1;
            while end < chars.len() && end < next_index + 3 {
                let Some(digit) = chars[end].to_digit(16) else { break };
                value = value * 16 + digit;
                end += 1;
            }
            if end == next_index + 1 {
                'x'
            } else {
                *index = end - 1;
                return char::from_u32(value);
            }
        }
        'u' | 'U' => {
            let digits = if escaped == 'u' { 4 } else { 8 };
            let start = next_index + 1;
            let end = start + digits;
            let value = chars
                .get(start..end)?
                .iter()
                .try_fold(0u32, |value, ch| ch.to_digit(16).map(|digit| value * 16 + digit))?;
            *index = end - 1;
            return char::from_u32(value);
        }
        '0'..='7' => {
            let mut value = escaped.to_digit(8)?;
            let mut end = next_index + 1;
            while end < chars.len() && end < next_index + 3 {
                let Some(digit) = chars[end].to_digit(8) else { break };
                value = value * 8 + digit;
                end += 1;
            }
            *index = end - 1;
            return char::from_u32(value);
        }
        _ => {
            // Bash preserves the backslash for unrecognized escapes. Emit it
            // here and let the caller append the escaped character next pass.
            return None;
        }
    };
    *index = next_index;
    Some(decoded)
}

/// Return the executable word of a simple command, skipping common prefix
/// assignments and transparent wrappers. This is policy-oriented and does not
/// attempt to emulate every option accepted by every wrapper.
pub(crate) fn executable(words: &[String]) -> Option<&str> {
    Some(words.get(executable_index(words)?)?.as_str())
}

/// Return the token index for the executable after common transparent
/// wrappers. Kept separate so policy checks can inspect its arguments without
/// reparsing a partially quoted command string.
pub(crate) fn executable_index(words: &[String]) -> Option<usize> {
    let mut i = 0usize;
    while words.get(i).is_some_and(|w| is_assignment(w)) {
        i += 1;
    }

    loop {
        let current = words.get(i)?.as_str();
        let base = current.rsplit('/').next().unwrap_or(current);
        match base {
            // `analyze` keeps shell reserved words with the simple command
            // that follows them (for example `then rm` / `do rm`). Skip the
            // command-introducing forms so policy examines that command.
            "do" | "then" | "else" | "elif" => i += 1,
            "case" => {
                i += 1;
                while words.get(i).is_some_and(|word| word != "in") {
                    i += 1;
                }
                if words.get(i).is_some_and(|word| word == "in") {
                    i += 1;
                }
                while words.get(i).is_some_and(|word| !word.ends_with(')')) {
                    i += 1;
                }
                if words.get(i).is_some_and(|word| word.ends_with(')')) {
                    i += 1;
                }
            }
            _ if current.ends_with(')') => i += 1,
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
            "nice" => {
                i += 1;
                while let Some(option) = words.get(i) {
                    if option == "--" {
                        i += 1;
                        break;
                    }
                    if option == "-n" || option == "--adjustment" {
                        i += 2;
                    } else if option.starts_with("--adjustment=")
                        || option
                            .strip_prefix("-n")
                            .is_some_and(|value| !value.is_empty() && value.parse::<f64>().is_ok())
                        || (option.starts_with('-') && option[1..].parse::<f64>().is_ok())
                    {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            "timeout" => {
                i += 1;
                while let Some(option) = words.get(i) {
                    if option == "--" {
                        i += 1;
                        break;
                    }
                    match option.as_str() {
                        "-k" | "--kill-after" | "-s" | "--signal" => i += 2,
                        "--foreground" | "--preserve-status" | "-v" | "--verbose" => i += 1,
                        _ if option.starts_with('-') => i += 1,
                        _ => {
                            // GNU timeout takes a duration before the command.
                            i += 1;
                            break;
                        }
                    }
                }
            }
            "stdbuf" => {
                i += 1;
                while let Some(option) = words.get(i) {
                    if option == "--" {
                        i += 1;
                        break;
                    }
                    if matches!(option.as_str(), "-i" | "-o" | "-e") {
                        i += 2;
                    } else if option.starts_with('-') {
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            "setsid" => {
                i += 1;
                while words.get(i).is_some_and(|w| w.starts_with('-') && w != "--") {
                    i += 1;
                }
                if words.get(i).is_some_and(|w| w == "--") {
                    i += 1;
                }
            }
            "sudo" => {
                i += 1;
                while let Some(option) = words.get(i) {
                    if option == "--" {
                        i += 1;
                        break;
                    }
                    if !option.starts_with('-') {
                        break;
                    }
                    let takes_value = matches!(
                        option.as_str(),
                        "-u" | "--user"
                            | "-g"
                            | "--group"
                            | "-h"
                            | "--host"
                            | "-p"
                            | "--prompt"
                            | "-C"
                            | "--close-from"
                            | "-T"
                            | "--command-timeout"
                            | "-R"
                            | "--chroot"
                            | "-D"
                            | "--chdir"
                    );
                    i += if takes_value { 2 } else { 1 };
                }
            }
            _ => return Some(i),
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

/// Return shell commands embedded in process substitutions (`<(...)` or
/// `>(...)`). They execute in a separate shell context, so policy checks must
/// inspect them even when the surrounding simple command is otherwise safe.
pub(crate) fn process_substitution_commands(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut result = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if quote == Some(b'\\') {
            quote = None;
            i += 1;
            continue;
        }
        if let Some(active_quote) = quote {
            if byte == active_quote {
                quote = None;
            } else if active_quote == b'"' && byte == b'\\' {
                quote = Some(b'\\');
            }
            i += 1;
            continue;
        }

        match byte {
            b'\\' => escaped = true,
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'<' | b'>' if bytes.get(i + 1) == Some(&b'(') => {
                let start = i + 2;
                if let Some(end) = substitution_paren_end(bytes, start) {
                    if let Some(body) = command.get(start..end) {
                        result.push(body);
                    }
                    i = end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    result
}

/// Return shell commands executed by command substitutions (`$(...)` and
/// backticks). Their output is data to the parent command, but the embedded
/// commands execute and must remain visible to security policy.
pub(crate) fn command_substitution_commands(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut result = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if quote == Some(b'\\') {
            quote = None;
            i += 1;
            continue;
        }

        if quote == Some(b'\'') {
            if byte == b'\'' {
                quote = None;
            }
            i += 1;
            continue;
        }

        if quote == Some(b'"') {
            match byte {
                b'"' => quote = None,
                b'\\' => quote = Some(b'\\'),
                b'$' if bytes.get(i + 1) == Some(&b'(') => {
                    let start = i + 2;
                    if let Some(end) = substitution_paren_end(bytes, start) {
                        if let Some(body) = command.get(start..end) {
                            result.push(body);
                        }
                        i = end + 1;
                        continue;
                    }
                }
                b'`' => {
                    if let Some(end) = backtick_substitution_end(bytes, i + 1) {
                        if let Some(body) = command.get(i + 1..end) {
                            result.push(body);
                        }
                        i = end + 1;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
            continue;
        }

        match byte {
            b'\\' => escaped = true,
            b'\'' | b'"' => quote = Some(byte),
            b'$' if bytes.get(i + 1) == Some(&b'(') => {
                let start = i + 2;
                if let Some(end) = substitution_paren_end(bytes, start) {
                    if let Some(body) = command.get(start..end) {
                        result.push(body);
                    }
                    i = end + 1;
                    continue;
                }
            }
            b'`' => {
                if let Some(end) = backtick_substitution_end(bytes, i + 1) {
                    if let Some(body) = command.get(i + 1..end) {
                        result.push(body);
                    }
                    i = end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    result
}

/// Return commands in shell brace groups, including function bodies and
/// grouped command lists. Checking their bodies is conservative: a function
/// may be invoked later even when the outer command only defines it.
pub(crate) fn brace_group_commands(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut result = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if quote == Some(b'\\') {
            quote = None;
            i += 1;
            continue;
        }
        if let Some(active_quote) = quote {
            if byte == active_quote {
                quote = None;
            } else if active_quote == b'"' && byte == b'\\' {
                quote = Some(b'\\');
            }
            i += 1;
            continue;
        }

        match byte {
            b'\\' => escaped = true,
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'{' if i == 0 || bytes[i - 1] != b'$' => {
                let start = i + 1;
                if let Some(end) = brace_group_end(bytes, start) {
                    if let Some(body) = command.get(start..end) {
                        result.push(body);
                    }
                    i = end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    result
}

/// Return shell commands in subshell groups (`(...)`). Command, process, and
/// arithmetic substitutions are handled by their dedicated extractors.
pub(crate) fn subshell_commands(command: &str) -> Vec<&str> {
    let bytes = command.as_bytes();
    let mut result = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if quote == Some(b'\\') {
            quote = None;
            i += 1;
            continue;
        }
        if let Some(active_quote) = quote {
            if byte == active_quote {
                quote = None;
            } else if active_quote == b'"' && byte == b'\\' {
                quote = Some(b'\\');
            }
            i += 1;
            continue;
        }

        match byte {
            b'\\' => escaped = true,
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'(' if !matches!(bytes.get(i.wrapping_sub(1)), Some(b'$' | b'<' | b'>'))
                && bytes.get(i + 1) != Some(&b'(') =>
            {
                let start = i + 1;
                if let Some(end) = substitution_paren_end(bytes, start) {
                    if let Some(body) = command.get(start..end) {
                        result.push(body);
                    }
                    i = end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    result
}

fn brace_group_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut quote = None;
    let mut escaped = false;
    let mut i = start;

    while i < bytes.len() {
        let byte = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if quote == Some(b'\\') {
            quote = None;
            i += 1;
            continue;
        }
        if let Some(active_quote) = quote {
            if byte == active_quote {
                quote = None;
            } else if active_quote == b'"' && byte == b'\\' {
                quote = Some(b'\\');
            }
            i += 1;
            continue;
        }

        match byte {
            b'\\' => escaped = true,
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'{' if i == 0 || bytes[i - 1] != b'$' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }

    None
}

fn backtick_substitution_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut escaped = false;
    for (index, byte) in bytes.iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
        } else if *byte == b'\\' {
            escaped = true;
        } else if *byte == b'`' {
            return Some(index);
        }
    }
    None
}

fn substitution_paren_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut quote = None;
    let mut escaped = false;
    let mut i = start;

    while i < bytes.len() {
        let byte = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if quote == Some(b'\\') {
            quote = None;
            i += 1;
            continue;
        }
        if let Some(active_quote) = quote {
            if byte == active_quote {
                quote = None;
            } else if active_quote == b'"' && byte == b'\\' {
                quote = Some(b'\\');
            } else if active_quote == b'"' && byte == b'(' && i > 0 && bytes[i - 1] == b'$' {
                depth += 1;
            } else if active_quote == b'"' && byte == b')' && depth > 1 {
                depth -= 1;
            }
            i += 1;
            continue;
        }

        match byte {
            b'\\' => escaped = true,
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
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
    fn extracts_process_substitution_commands_without_matching_quoted_parens() {
        assert_eq!(
            process_substitution_commands("diff <(printf ')'; rm -rf /) <(cat file)"),
            vec!["printf ')'; rm -rf /", "cat file"]
        );
        assert!(process_substitution_commands("echo '<(rm -rf /)'").is_empty());
    }

    #[test]
    fn extracts_command_substitution_commands_without_matching_quoted_parens() {
        assert_eq!(
            command_substitution_commands(r#"echo "$(printf ')'; rm x)" `printf y; rm z`"#),
            vec!["printf ')'; rm x", "printf y; rm z"]
        );
        assert!(command_substitution_commands("echo '$(rm -rf /)'").is_empty());
    }

    #[test]
    fn extracts_brace_groups_without_matching_quoted_braces() {
        assert_eq!(
            brace_group_commands("f() { echo '}'; cmd=rm; $cmd -rf /; } { cat file; }"),
            vec![" echo '}'; cmd=rm; $cmd -rf /; ", " cat file; "]
        );
        assert!(brace_group_commands("echo '{ not shell code }'").is_empty());
    }

    #[test]
    fn extracts_subshell_commands_without_matching_quoted_parens() {
        assert_eq!(
            subshell_commands("(echo ')'; cmd=rm; $cmd -rf /) (cat file)"),
            vec!["echo ')'; cmd=rm; $cmd -rf /", "cat file"]
        );
        assert!(subshell_commands("echo '(rm -rf /)'").is_empty());
        assert!(subshell_commands("echo $(printf safe)").is_empty());
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
    fn normalizes_line_continuations_and_ansi_c_quotes() {
        let shape = analyze("r\\\nm -fr /");
        assert_eq!(shape.commands, vec![vec!["rm", "-fr", "/"]]);
        let ansi_quoted = analyze("$'\\x72m' -fr /");
        assert_eq!(ansi_quoted.commands, vec![vec!["rm", "-fr", "/"]]);
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
