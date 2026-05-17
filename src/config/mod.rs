//! Configuration system — TOML file, env vars, CLI overrides.
//!
//! Merge priority (highest wins): CLI args > env vars > config file > defaults.

mod merge;
mod schema;

pub use schema::*;

use crate::Result;
use std::path::{Path, PathBuf};

/// CLI-settable overrides applied on top of all other config sources.
#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    pub log_level: Option<String>,
    pub socket_path: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
}

impl Config {
    /// Load config from all sources in priority order.
    pub fn load(overrides: CliOverrides) -> Result<Self> {
        let mut cfg = Self::default();

        // 1. Config file
        let config_path = overrides
            .config_path
            .clone()
            .or_else(default_config_path);

        if let Some(path) = &config_path {
            if path.exists() {
                let content = std::fs::read_to_string(path)
                    .map_err(|e| crate::ArshyError::Config(format!("read config: {}", e)))?;
                let partial: PartialConfig = toml::from_str(&content)?;
                cfg = merge::merge_partial(cfg, partial);
            }
        }

        // 2. Environment variables
        merge::apply_env(&mut cfg);

        // 3. CLI overrides
        merge::apply_cli(&mut cfg, &overrides);

        // 4. Version check — warn if config was written for a newer schema
        if cfg.version > 1 {
            tracing::warn!(
                "config version {} is newer than this build (version 1). \
                 Some settings may not be recognized. Consider upgrading arshy.",
                cfg.version
            );
        }

        Ok(cfg)
    }

    /// Write the current config to the default config file path.
    pub fn write_to_default_path(&self) -> Result<()> {
        let path = default_config_path()
            .ok_or_else(|| crate::ArshyError::Config("no config dir".into()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}

/// Returns ~/.config/arshy/config.toml
pub fn default_config_path() -> Option<PathBuf> {
    Some(PathBuf::from(xdg_config_home()).join("arshy").join("config.toml"))
}

/// XDG_CONFIG_HOME (default ~/.config)
pub fn xdg_config_home() -> String {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| home_join(".config"))
}

/// XDG_DATA_HOME (default ~/.local/share)
pub fn xdg_data_home() -> String {
    std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| home_join(".local/share"))
}

/// XDG_CACHE_HOME (default ~/.cache)
pub fn xdg_cache_home() -> String {
    std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| home_join(".cache"))
}

fn home_join(suffix: &str) -> String {
    dirs::home_dir()
        .map(|p| p.join(suffix).to_string_lossy().to_string())
        .unwrap_or_else(|| format!("~{}", suffix))
}

/// Expand `${XDG_DATA_HOME}`, `${XDG_CONFIG_HOME}`, `${XDG_CACHE_HOME}`, `${HOME}`
/// in a path string.
pub fn expand_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let expanded = s
        .replace("${XDG_DATA_HOME}", &xdg_data_home())
        .replace("${XDG_CONFIG_HOME}", &xdg_config_home())
        .replace("${XDG_CACHE_HOME}", &xdg_cache_home())
        .replace("${HOME}", &dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "~".into()));
    PathBuf::from(expanded)
}

/// Initialize tracing/logging globally.
pub fn init_logging(level: &str, format: &str) {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    if format == "json" {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json().with_target(true))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().compact().with_target(true))
            .init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_compiles() {
        let cfg = Config::default();
        assert_eq!(cfg.daemon.log_level, "info");
        assert!(cfg.store.wal_mode);
    }

    #[test]
    fn default_config_all_sections() {
        let cfg = Config::default();
        // Daemon
        assert!(cfg.daemon.auto_start);
        assert_eq!(cfg.daemon.max_concurrent_tasks, 4);
        assert_eq!(cfg.daemon.max_output_bytes, 10_485_760);
        // Store
        assert!(cfg.store.integrity_check);
        assert_eq!(cfg.store.prune_keep, 1000);
        assert_eq!(cfg.store.prune_older_than_days, 30);
        // Parser
        assert!(cfg.parser.hot_reload);
        assert!(cfg.parser.fallback_to_raw);
        assert_eq!(cfg.parser.default_priority, 50);
        // Notifications
        assert!(cfg.notifications.enabled);
        assert_eq!(cfg.notifications.batch_interval_ms, 100);
        // MCP
        assert!(!cfg.mcp.client_detection_order.is_empty());
    }

    #[test]
    fn expand_path_with_home() {
        let home = dirs::home_dir().unwrap();
        let result = expand_path(&PathBuf::from("${HOME}/.arshy/parsers"));
        assert!(result.starts_with(&home));
    }

    #[test]
    fn expand_path_with_xdg_data_home() {
        let xdg = xdg_data_home();
        let result = expand_path(&PathBuf::from("${XDG_DATA_HOME}/arshy/arshy.db"));
        assert!(result.to_string_lossy().starts_with(&xdg));
        assert!(result.to_string_lossy().ends_with("arshy/arshy.db"));
    }

    #[test]
    fn expand_path_no_placeholder() {
        let result = expand_path(&PathBuf::from("/tmp/plain/path.db"));
        assert_eq!(result, PathBuf::from("/tmp/plain/path.db"));
    }

    #[test]
    fn expand_path_multiple_placeholders() {
        let home = dirs::home_dir().unwrap();
        let result = expand_path(&PathBuf::from("${HOME}/${HOME}"));
        let expected = format!("{}/{}", home.display(), home.display());
        assert_eq!(result, PathBuf::from(expected));
    }

    #[test]
    fn xdg_defaults_are_nonempty() {
        // These should always return non-empty paths
        assert!(!xdg_config_home().is_empty());
        assert!(!xdg_data_home().is_empty());
        assert!(!xdg_cache_home().is_empty());
    }

    #[test]
    fn default_config_path_exists() {
        let path = default_config_path().unwrap();
        assert!(path.to_string_lossy().contains("arshy"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn expanded_socket_path_is_absolute() {
        let cfg = Config::default();
        let expanded = cfg.daemon.expanded_socket_path();
        // After expansion, no ${} placeholders should remain
        let s = expanded.to_string_lossy();
        assert!(!s.contains("${"), "path still has placeholder: {}", s);
    }

    #[test]
    fn expanded_db_path_is_absolute() {
        let cfg = Config::default();
        let expanded = cfg.store.expanded_db_path();
        let s = expanded.to_string_lossy();
        assert!(!s.contains("${"), "path still has placeholder: {}", s);
    }
}
