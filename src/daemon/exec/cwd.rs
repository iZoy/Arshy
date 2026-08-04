//! CWD accessibility — macOS TCC detection and `/tmp/.arshy-cwd` symlink fallback.

use std::collections::HashMap;

// ── CWD Accessibility & Symlink Fallback ───────────────────────────────

/// Test if a directory is likely restricted by macOS TCC (Transparency, Consent, and Control).
/// TCC restricts PTY child processes from accessing certain top-level directories
/// (~/Documents, ~/Desktop, ~/Downloads) even when the parent daemon can write there.
///
/// On non-macOS platforms, this always returns false.
fn is_tcc_restricted(path: &std::path::Path) -> bool {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        let home = match dirs::home_dir() {
            Some(h) => h,
            None => return false,
        };
        // Only check top-level directories that macOS TCC protects
        let Ok(relative) = path.strip_prefix(&home) else {
            return false;
        };
        let top_dir =
            relative.components().next().and_then(|c| c.as_os_str().to_str()).unwrap_or("");
        matches!(top_dir, "Documents" | "Desktop" | "Downloads")
    }
}

/// Create a symlink at `/tmp/.arshy-cwd/<hash>` pointing to `real_cwd`.
/// This bypasses macOS TCC restrictions because symlinks inherit the parent
/// directory's permissions, not the target's.
///
/// Returns the symlink path (to use as PTY cwd) if successful.
fn create_cwd_symlink(real_cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    real_cwd.hash(&mut hasher);
    let hash = format!("{:016x}", hasher.finish());

    let symlink_dir = std::path::PathBuf::from("/tmp/.arshy-cwd");
    let symlink_path = symlink_dir.join(&hash);

    std::fs::create_dir_all(&symlink_dir).ok()?;

    // Remove stale symlink if it points somewhere else
    if symlink_path.exists() || symlink_path.symlink_metadata().is_ok() {
        let _ = std::fs::remove_file(&symlink_path);
    }

    std::os::unix::fs::symlink(real_cwd, &symlink_path).ok()?;
    Some(symlink_path)
}

/// Prepare a working directory for PTY execution. If the target directory is
/// in a macOS TCC-restricted location, creates a symlink fallback and injects
/// ARSHY_CWD into the environment so the command knows the real path.
///
/// Returns (effective_cwd, fallback_path_to_cleanup).
pub(crate) fn prepare_cwd(
    real_cwd: &std::path::Path,
    env: &mut Option<HashMap<String, String>>,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    // Fast path: directory is not in a TCC-restricted location
    if !is_tcc_restricted(real_cwd) {
        return (None, None);
    }

    // Directory is in a TCC-restricted location — create symlink fallback
    tracing::info!(
        "cwd {:?} is in a TCC-restricted directory, creating symlink fallback",
        real_cwd
    );

    let Some(symlink) = create_cwd_symlink(real_cwd) else {
        tracing::warn!("symlink fallback failed for {:?}", real_cwd);
        return (None, None);
    };

    // Inject ARSHY_CWD so the command can find the real project directory
    let env_map = env.get_or_insert_with(HashMap::new);
    env_map.insert("ARSHY_CWD".to_string(), real_cwd.to_string_lossy().to_string());

    tracing::info!("cwd fallback: {:?} -> {:?}", real_cwd, symlink);
    (Some(symlink.clone()), Some(symlink))
}
