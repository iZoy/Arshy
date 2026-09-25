//! CWD accessibility — macOS TCC path detection and a private symlink fallback.

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
        false
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

/// Create a private, per-execution symlink to `real_cwd`.
///
/// The symlink provides an alternate path name; it does not change filesystem
/// permissions. Whether it changes macOS TCC behavior for the spawned process
/// has not been verified and must not be treated as a security boundary.
///
/// Returns the symlink path (to use as PTY cwd) if successful.
fn create_cwd_symlink(real_cwd: &std::path::Path) -> Option<std::path::PathBuf> {
    // Use a per-user directory and reject pre-existing paths owned by another
    // user or symlinks. `/tmp` is shared, so create_dir_all alone is unsafe.
    // SAFETY: geteuid has no preconditions and only reads the effective uid.
    let uid = unsafe { libc::geteuid() };
    let symlink_dir = std::path::PathBuf::from(format!("/tmp/.arshy-cwd-{uid}"));
    create_private_cwd_symlink(&symlink_dir, real_cwd, uid)
}

fn create_private_cwd_symlink(
    symlink_dir: &std::path::Path,
    real_cwd: &std::path::Path,
    uid: u32,
) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match std::fs::DirBuilder::new().mode(0o700).create(symlink_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }

    let metadata = std::fs::symlink_metadata(symlink_dir).ok()?;
    if !metadata.file_type().is_dir() || metadata.uid() != uid {
        tracing::warn!("refusing unsafe cwd symlink directory {:?}", symlink_dir);
        return None;
    }
    std::fs::set_permissions(symlink_dir, std::fs::Permissions::from_mode(0o700)).ok()?;

    // A fresh name prevents concurrent commands from replacing or cleaning up
    // each other's cwd link. The private directory limits access to this uid.
    let symlink_path = symlink_dir.join(uuid::Uuid::new_v4().to_string());
    std::os::unix::fs::symlink(real_cwd, &symlink_path).ok()?;
    Some(symlink_path)
}

/// Prepare a working directory for PTY execution. If the target directory is
/// in a macOS TCC-protected location, creates a symlink path alias and injects
/// ARSHY_CWD so the command can discover the requested path. This does not
/// guarantee that macOS TCC will grant access through the alias.
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

    // Directory matches the TCC-protected path heuristic — create a path alias.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    #[test]
    fn cwd_fallback_uses_private_directory_and_unique_links() {
        let target = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("private-cwd");
        // SAFETY: geteuid has no preconditions and only reads the effective uid.
        let uid = unsafe { libc::geteuid() };
        let first = create_private_cwd_symlink(&directory, target.path(), uid).unwrap();
        let second = create_private_cwd_symlink(&directory, target.path(), uid).unwrap();

        assert_ne!(first, second, "concurrent executions need distinct cwd links");
        assert_eq!(std::fs::read_link(&first).unwrap(), target.path());
        assert_eq!(std::fs::read_link(&second).unwrap(), target.path());

        let metadata = std::fs::symlink_metadata(&directory).unwrap();
        // SAFETY: geteuid has no preconditions and only reads the effective uid.
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700);

        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
