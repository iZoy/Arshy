//! Error context extraction — reads source files around error locations.

use arshy_lib::ipc::EventContext;

/// Extract ±3 lines of source context around `line` in `file`.
/// Returns None if the file can't be read or the line is out of range.
#[allow(dead_code)] // sync version; async variant used in production
pub fn extract_context(file: &str, line: u64) -> Option<EventContext> {
    let content = std::fs::read_to_string(file).ok()?;
    parse_context(&content, line)
}

/// Async version — reads the file using tokio::fs to avoid blocking.
pub async fn extract_context_async(file: &str, line: u64) -> Option<EventContext> {
    let content = tokio::fs::read_to_string(file).await.ok()?;
    parse_context(&content, line)
}

/// Parse context from file content string. Shared between sync and async paths.
fn parse_context(content: &str, line: u64) -> Option<EventContext> {
    let lines: Vec<&str> = content.lines().collect();
    let idx = (line as usize).saturating_sub(1);

    if idx >= lines.len() {
        return None;
    }

    let before: Vec<String> = lines[idx.saturating_sub(3)..idx]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let after: Vec<String> = lines[idx + 1..(idx + 4).min(lines.len())]
        .iter()
        .map(|s| s.to_string())
        .collect();

    Some(EventContext {
        before,
        line: lines[idx].to_string(),
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_middle_line() {
        let ctx = extract_context("Cargo.toml", 1).unwrap();
        assert!(!ctx.line.is_empty());
    }

    #[test]
    fn out_of_range_returns_none() {
        assert!(extract_context("Cargo.toml", 99999).is_none());
    }

    #[test]
    fn missing_file_returns_none() {
        assert!(extract_context("/nonexistent/file.txt", 1).is_none());
    }

    #[tokio::test]
    async fn async_extract_context() {
        let ctx = extract_context_async("Cargo.toml", 1).await.unwrap();
        assert!(!ctx.line.is_empty());
    }

    #[tokio::test]
    async fn async_missing_file_returns_none() {
        assert!(extract_context_async("/nonexistent/file.txt", 1).await.is_none());
    }
}
