//! Lightweight ReDoS (Regular Expression Denial of Service) safety validator.
//!
//! Checks for common patterns that cause catastrophic backtracking:
//! - Nested quantifiers: `(?:a+)+`, `(?:a*)*`, `(?:a{1,5}){2,10}`
//! - Overlapping alternation with quantifiers: `(?:a|ab)+` followed by fail
//!
//! This is a static analysis pass, not a runtime fuzzer. It runs once at regex
//! compile time during parser loading. Patterns that fail are rejected with a
//! clear error message — they never reach production.

/// Check a regex string for ReDoS patterns.
/// Returns `Ok(())` if the regex passes, or `Err(reason)` if it's dangerous.
pub fn check_safe(regex: &str) -> Result<(), String> {
    // 1. Nested quantifiers: look for a quantified group that itself contains
    //    a quantifier — e.g. (?:a+)+, (?:.*)+, (?:a{1,5}){2,10}
    if has_nested_quantifier(regex) {
        return Err(format!(
            "ReDoS: nested quantifier detected in '{}'. \
             Nested quantifiers cause exponential backtracking. \
             Restructure the pattern to avoid quantifier nesting.",
            regex
        ));
    }

    // 2. Overlapping alternation with trailing unbounded repeat
    //    Pattern like `(a|ab)+b` on input "ab" causes backtracking
    if has_overlapping_alternation(regex) {
        return Err(format!(
            "ReDoS: overlapping alternation with quantifier in '{}'. \
             Branches that share a prefix (a|ab) combined with a repeat \
             can cause exponential backtracking on non-matching input.",
            regex
        ));
    }

    Ok(())
}

/// Detect nested quantifiers: a repeat operator applied to a group that
/// already contains a repeat operator.
///
/// Matches patterns like:
/// - `(?:a+)+` — nested `+`
/// - `(?:a*)*` — nested `*`
/// - `(?:a{1,5}){2,10}` — nested `{}`
/// - `(?:.+)+` — `.` with nested `+`
fn has_nested_quantifier(regex: &str) -> bool {
    let bytes = regex.as_bytes();
    let len = bytes.len();
    let mut depth: u32 = 0;
    let mut group_has_quantifier = Vec::new(); // stack: does current group contain a quantifier?

    let mut i = 0;
    while i < len {
        let ch = bytes[i];
        match ch {
            b'(' => {
                group_has_quantifier.push(false);
                depth += 1;
                i += 1;
            }
            b')' => {
                if depth > 0 {
                    let had_quant = group_has_quantifier.pop().unwrap_or(false);
                    // Check if this group is followed by a quantifier
                    if i + 1 < len {
                        let next = bytes[i + 1];
                        if (next == b'+' || next == b'*' || next == b'{') && had_quant {
                            return true;
                        }
                    }
                    depth -= 1;
                }
                i += 1;
            }
            b'+' | b'*' | b'{' => {
                // Mark the current innermost group as having a quantifier
                if let Some(last) = group_has_quantifier.last_mut() {
                    *last = true;
                }
                // Skip over {n,m} to avoid false matching on internal commas
                if ch == b'{' {
                    while i < len && bytes[i] != b'}' {
                        i += 1;
                    }
                }
                i += 1;
            }
            b'\\' => {
                // Skip escaped character
                i += 2;
            }
            b'[' => {
                // Skip character class
                while i < len && bytes[i] != b']' {
                    if bytes[i] == b'\\' { i += 1; }
                    i += 1;
                }
                i += 1; // skip ']'
            }
            _ => {
                i += 1;
            }
        }
    }
    false
}

/// Detect overlapping alternation: `(?:a|ab)` where branches share a prefix.
/// This causes backtracking when the first branch matches partially and fails.
///
/// Only flags when the alternation is inside a quantified group, e.g.:
/// `(?:a|ab)+b` — dangerous
/// `(?:a|ab)` — harmless (no quantifier on the group)
fn has_overlapping_alternation(regex: &str) -> bool {
    // Find alternation groups: `(?: ... | ... )+` or `(?: ... | ... )*`
    // that have one branch being a prefix of another
    let bytes = regex.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Look for `(?:`
        if i + 3 < len && bytes[i] == b'(' && bytes[i + 1] == b'?' && bytes[i + 2] == b':' {
            let group_start = i + 3;
            let mut group_end = group_start;
            let mut depth: u32 = 0;
            while group_end < len {
                match bytes[group_end] {
                    b'(' => depth += 1,
                    b')' if depth == 0 => break,
                    b')' => depth -= 1,
                    _ => {}
                }
                group_end += 1;
            }
            let group_content = &regex[group_start..group_end];
            let is_quantified = group_end + 1 < len
                && matches!(bytes[group_end + 1], b'+' | b'*' | b'{');

            if is_quantified && group_content.contains('|') {
                // Collect alternation branches
                let branches: Vec<&str> = group_content.split('|').collect();
                for j in 0..branches.len() {
                    for k in 0..branches.len() {
                        if j != k && !branches[j].is_empty()
                            && branches[k].starts_with(branches[j])
                        {
                            return true;
                        }
                    }
                }
            }
            i = group_end + 1;
        } else {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Safe patterns ──────────────────────────────────────────────────────

    #[test]
    fn safe_simple() {
        assert!(check_safe(r"^error: (.+)$").is_ok());
        assert!(check_safe(r"^\s+at .+ \((.+):(\d+):\d+\)").is_ok());
        assert!(check_safe(r"^(.+?)\((\d+),(\d+)\): error (TS\d+): (.+)$").is_ok());
    }

    #[test]
    fn safe_builtin_cargo() {
        assert!(check_safe(r"^error\[([E]\d+)\]: (.+)$").is_ok());
        assert!(check_safe(r"^\s*--> (.+?):(\d+):(\d+)$").is_ok());
        assert!(check_safe(r"^warning: (.+)$").is_ok());
    }

    #[test]
    fn safe_alternation_without_quantifier() {
        // Alternation without quantifier on the group should be fine
        assert!(check_safe(r"^(error|warning|info)$").is_ok());
    }

    #[test]
    fn safe_character_class() {
        assert!(check_safe(r"^[a-zA-Z0-9_]+$").is_ok());
        assert!(check_safe(r"^\[(\d+)\]").is_ok());
    }

    #[test]
    fn safe_escaped_parens() {
        assert!(check_safe(r"^\((\d+)\)").is_ok());
    }

    // ── Dangerous patterns ─────────────────────────────────────────────────

    #[test]
    fn reject_nested_plus() {
        assert!(check_safe(r"^(a+)+$").is_err());
    }

    #[test]
    fn reject_nested_star() {
        assert!(check_safe(r"^(a*)*$").is_err());
    }

    #[test]
    fn reject_nested_repeat() {
        assert!(check_safe(r"^(a{1,5}){2,10}$").is_err());
    }

    #[test]
    fn reject_nested_plus_in_group() {
        assert!(check_safe(r"^(?:a+)+$").is_err());
    }

    #[test]
    fn reject_dot_plus_nested() {
        // (.+)+ — the classic evil regex
        assert!(check_safe(r"^(.+)+$").is_err());
    }

    #[test]
    fn reject_overlapping_alternation_quantified() {
        // (?:a|ab)+ on input "ab" — dangerous
        assert!(check_safe(r"^(?:a|ab)+b$").is_err());
    }
}
