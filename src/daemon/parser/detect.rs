/// Extract the tool name from a command string.
/// "npm run build" → "npm", "npx tsc" → "npx"
pub fn extract_tool(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or(command)
}

/// Build a version detection command for common tools.
pub fn version_command(tool: &str) -> Option<String> {
    match tool {
        "tsc" => Some("npx tsc --version".into()),
        "cargo" => Some("cargo --version".into()),
        "go" => Some("go version".into()),
        "node" => Some("node --version".into()),
        "npm" => Some("npm --version".into()),
        "rustc" => Some("rustc --version".into()),
        _ => None,
    }
}

/// Parse a semver-like version from command output.
pub fn parse_version(output: &str) -> Option<String> {
    let re = regex::Regex::new(r"\d+\.\d+\.\d+").ok()?;
    re.find(output).map(|m| m.as_str().to_string())
}
