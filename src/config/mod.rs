//! Configuration system — TOML file, env vars, CLI overrides.
//!
//! Merge priority (highest wins): CLI args > env vars > config file > defaults.

mod merge;
mod schema;

pub use schema::*;

use crate::Result;
use std::path::PathBuf;

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
pub fn expand_path(path: &PathBuf) -> PathBuf {
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
    fn test_default_config_compiles() {
        let cfg = Config::default();
        assert_eq!(cfg.daemon.log_level, "info");
        assert!(cfg.store.wal_mode);
    }

    #[test]
    fn test_expand_path_with_home() {
        let home = dirs::home_dir().unwrap();
        let result = expand_path(&PathBuf::from("${HOME}/.arshy/parsers"));
        assert!(result.starts_with(&home));
    }
}
