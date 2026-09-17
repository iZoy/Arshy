//! Developer self-update helper.
//!
//! Installation remains generic; Agent onboarding is handled by the
//! client-neutral MCP setup Prompt and never edits shell profiles or PATH entries.

use arshy_lib::Result;
use std::path::PathBuf;

pub(crate) fn self_update(dest: Option<PathBuf>) -> Result<()> {
    let exe = std::env::current_exe()?;
    let arshyd = exe.parent().map(|p| p.join("arshyd")).filter(|p| p.is_file());
    let dest_dir = dest.unwrap_or_else(|| default_install_dir(&exe));
    std::fs::create_dir_all(&dest_dir)?;

    let mut copied = Vec::new();
    let dest_arshy = dest_dir.join("arshy");
    if dest_arshy != exe {
        std::fs::copy(&exe, &dest_arshy)?;
        copied.push(dest_arshy);
    }
    if let Some(source) = arshyd {
        let destination = dest_dir.join("arshyd");
        if destination != source {
            std::fs::copy(source, &destination)?;
            copied.push(destination);
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in &copied {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    if copied.is_empty() {
        println!("arshy is already up to date at {}", dest_dir.display());
    } else {
        for path in copied {
            println!("  ✓ updated {}", path.display());
        }
        println!("Restart the daemon to pick up the new binary: `arshy daemon restart`");
    }
    Ok(())
}

pub(crate) fn default_install_dir(exe: &std::path::Path) -> PathBuf {
    let in_target = exe.components().any(|c| c.as_os_str() == "target");
    if in_target {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("~")).join(".local").join("bin")
    } else {
        exe.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
    }
}
