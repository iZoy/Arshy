//! Diagnostic selection helpers.

/// Select a representative error-level diagnostic from the complete event set.
pub(crate) fn select_primary_diagnostic(
    events: &Option<Vec<serde_json::Value>>,
) -> Option<serde_json::Value> {
    let evts = events.as_ref()?;
    let first =
        evts.iter().find(|e| e.get("severity").and_then(|v| v.as_str()) == Some("error"))?;
    // Traceback streams start with a banner; the final diagnostic is the
    // exception that explains the failure.
    let is_traceback_banner = first
        .get("message")
        .and_then(|v| v.as_str())
        .is_some_and(|message| message.trim_start().starts_with("Traceback ("));
    if is_traceback_banner {
        return evts
            .iter()
            .rev()
            .find(|event| {
                event.get("severity").and_then(|v| v.as_str()) == Some("error")
                    && event.get("type").and_then(|v| v.as_str()) == Some("diagnostic")
            })
            .cloned()
            .or(Some(first.clone()));
    }
    Some(first.clone())
}
