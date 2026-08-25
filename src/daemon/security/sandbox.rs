//! Working-directory guard — restricts cwd to explicitly allowed roots.

use crate::{ArshyError, Result};
use std::path::PathBuf;

/// Check that `cwd` is within one of the allowed cwd roots.
///
/// - If the roots are empty, the check is skipped (permissive default).
/// - Rejects `../` escapes by canonicalizing both sides.
/// - Rejects symlink escapes by resolving to real paths.
pub fn check_path(cwd: &str, allowed_cwds: &[String]) -> Result<()> {
    if allowed_cwds.is_empty() {
        return Ok(());
    }

    let cwd_path = PathBuf::from(cwd);
    let canonical_cwd = cwd_path
        .canonicalize()
        .map_err(|_| ArshyError::Ipc(format!("invalid cwd: cannot resolve '{}'", cwd)))?;

    for root in allowed_cwds {
        let root_path = PathBuf::from(expand_tilde(root));
        let canonical_root = match root_path.canonicalize() {
            Ok(p) => p,
            Err(_) => continue, // sandbox path doesn't exist, skip
        };

        if canonical_cwd.starts_with(&canonical_root) {
            return Ok(());
        }
    }

    Err(ArshyError::Ipc(format!("access denied: cwd '{}' is outside allowed roots", cwd)))
}

/// Expand `~` to the home directory.
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs::home_dir() {
            return path.replacen('~', &home.to_string_lossy(), 1);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn empty_sandbox_allows_anything() {
        assert!(check_path("/tmp", &[]).is_ok());
        assert!(check_path("/anywhere", &[]).is_ok());
    }

    #[test]
    fn path_inside_sandbox_passes() {
        let tmp = TempDir::new().unwrap();
        let sandbox = tmp.path().to_string_lossy().to_string();
        let cwd = tmp.path().to_string_lossy().to_string();
        assert!(check_path(&cwd, &[sandbox]).is_ok());
    }

    #[test]
    fn path_outside_sandbox_rejected() {
        let tmp = TempDir::new().unwrap();
        let sandbox = tmp.path().to_string_lossy().to_string();
        assert!(check_path("/tmp", &[sandbox]).is_err());
    }

    #[test]
    fn dotdot_escape_rejected() {
        let tmp = TempDir::new().unwrap();
        let inner = tmp.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        let sandbox = inner.to_string_lossy().to_string();

        // Trying to escape via ../ should fail
        let escape = inner.join("..").to_string_lossy().to_string();
        assert!(check_path(&escape, &[sandbox]).is_err());
    }

    #[test]
    fn symlink_escape_rejected() {
        let tmp = TempDir::new().unwrap();
        let sandbox_dir = tmp.path().join("sandbox");
        std::fs::create_dir(&sandbox_dir).unwrap();
        let outside_dir = tmp.path().join("outside");
        std::fs::create_dir(&outside_dir).unwrap();

        // Create a symlink inside sandbox pointing outside
        let symlink = sandbox_dir.join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_dir, &symlink).unwrap();

        let sandbox = sandbox_dir.to_string_lossy().to_string();
        let symlink_path = symlink.to_string_lossy().to_string();

        // The symlink resolves to outside_dir, which is outside sandbox
        assert!(check_path(&symlink_path, &[sandbox]).is_err());
    }

    #[test]
    fn multiple_allowed_cwds() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let sandboxes = vec![
            tmp1.path().to_string_lossy().to_string(),
            tmp2.path().to_string_lossy().to_string(),
        ];

        assert!(check_path(&tmp1.path().to_string_lossy(), &sandboxes).is_ok());
        assert!(check_path(&tmp2.path().to_string_lossy(), &sandboxes).is_ok());
        assert!(check_path("/tmp", &sandboxes).is_err());
    }

    #[test]
    fn invalid_cwd_rejected() {
        assert!(check_path("/nonexistent/path/that/does/not/exist", &["/tmp".into()]).is_err());
    }

    #[test]
    fn tilde_expansion() {
        // ~/ expands to home directory
        let result = expand_tilde("~/Documents");
        assert!(!result.starts_with('~'));
        assert!(result.ends_with("Documents"));
    }
}
