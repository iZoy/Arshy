//! Tool detection and version probing.
//!
//! Extracts the tool name from a command, probes its version (with caching),
//! and provides version strings for parser matching.

/// Extract the tool name from a command string.
/// "npm run build" → "npm", "npx tsc" → "npx"
#[allow(dead_code)] // utility; currently detection done via registry.detect()
pub fn extract_tool(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or(command)
}

/// Build a version detection command for common tools.
pub fn version_command(tool: &str) -> Option<&'static str> {
    match tool {
        "tsc" => Some("npx tsc --version"),
        "cargo" => Some("cargo --version"),
        "go" => Some("go version"),
        "node" => Some("node --version"),
        "npm" => Some("npm --version"),
        "pnpm" => Some("pnpm --version"),
        "rustc" => Some("rustc --version"),
        "python" | "python3" => Some("python3 --version"),
        "gcc" => Some("gcc --version"),
        "clang" => Some("clang --version"),
        "docker" => Some("docker --version"),
        "terraform" => Some("terraform --version"),
        "gradle" => Some("gradle --version"),
        "make" => Some("make --version"),
        "pip" => Some("pip --version"),
        _ => None,
    }
}

/// Parse a semver-like version from command output.
/// Matches patterns like "1.2.3", "v1.2.3", "version 1.2.3".
pub fn parse_version(output: &str) -> Option<String> {
    use std::sync::LazyLock;
    static VERSION_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\d+\.\d+\.\d+").unwrap());
    VERSION_RE.find(output).map(|m| m.as_str().to_string())
}

/// Check whether `actual` satisfies `min <= actual <= max`.
/// Each version is split on '.' and compared component-wise.
/// Returns true if `min` and `max` are both None (no constraint).
pub fn version_satisfies(actual: &str, min: Option<&str>, max: Option<&str>) -> bool {
    if min.is_none() && max.is_none() {
        return true;
    }
    let actual_parts: Vec<u32> = actual.split('.').filter_map(|s| s.parse().ok()).collect();
    if actual_parts.is_empty() {
        return true; // can't parse actual version — don't filter
    }
    if let Some(min_v) = min {
        let min_parts: Vec<u32> = min_v.split('.').filter_map(|s| s.parse().ok()).collect();
        if !min_parts.is_empty() && actual_parts < min_parts {
            return false;
        }
    }
    if let Some(max_v) = max {
        let max_parts: Vec<u32> = max_v.split('.').filter_map(|s| s.parse().ok()).collect();
        if !max_parts.is_empty() && actual_parts > max_parts {
            return false;
        }
    }
    true
}

/// Probe the version of a tool, using the cache if available.
///
/// Returns `(version, from_cache)` where `from_cache` is true if the version
/// was read from the store cache rather than by running the tool.
pub async fn probe_version(
    tool: &str,
    store: &super::super::store::Store,
    cache_ttl_hours: u64,
) -> Option<String> {
    // 1. Check cache
    if let Ok(Some(version)) = store.get_cached_version(tool) {
        tracing::debug!("tool '{}' version {} from cache", tool, version);
        return Some(version);
    }

    // 2. Run version command
    let cmd = version_command(tool)?;
    tracing::debug!("probing version for '{}' via '{}'", tool, cmd);

    let output = tokio::process::Command::new("sh").args(["-c", cmd]).output().await.ok()?;

    if !output.status.success() {
        tracing::warn!("version probe for '{}' failed (exit {})", tool, output.status);
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    let version = parse_version(&combined)?;

    // 3. Cache result
    if let Err(e) = store.cache_version(tool, &version, cache_ttl_hours) {
        tracing::warn!("failed to cache version for '{}': {}", tool, e);
    }

    tracing::info!("tool '{}' version {} detected", tool, version);
    Some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tool() {
        assert_eq!(extract_tool("npm run build"), "npm");
        assert_eq!(extract_tool("npx tsc --noEmit"), "npx");
        assert_eq!(extract_tool("cargo test"), "cargo");
        assert_eq!(extract_tool("echo hello"), "echo");
    }

    #[test]
    fn test_version_command() {
        assert_eq!(version_command("npm"), Some("npm --version"));
        assert_eq!(version_command("cargo"), Some("cargo --version"));
        assert_eq!(version_command("tsc"), Some("npx tsc --version"));
        assert_eq!(version_command("unknown_tool"), None);
    }

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("10.2.3"), Some("10.2.3".into()));
        assert_eq!(parse_version("v1.2.3"), Some("1.2.3".into()));
        assert_eq!(parse_version("node v20.10.0"), Some("20.10.0".into()));
        assert_eq!(parse_version("cargo 1.75.0 (1d02220cf 2023-12-04)"), Some("1.75.0".into()));
        assert_eq!(parse_version("no version here"), None);
    }
}
