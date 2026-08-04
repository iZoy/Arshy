//! Tool detection helpers.
//!
//! Version-constraint satisfaction for parser TOML specs (min/max version).

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
