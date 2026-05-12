use thiserror::Error;

#[derive(Error, Debug)]
pub enum ArshyError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("daemon unreachable: {0}")]
    DaemonUnreachable(String),

    #[error("task not found: {0}")]
    TaskNotFound(String),

    #[error("task exited with code {exit_code}: {message}")]
    TaskExecution { exit_code: i32, message: String },

    #[error("task timed out after {duration_ms}ms")]
    TaskTimeout { duration_ms: u64 },

    #[error("parser error: {0}")]
    Parser(String),

    #[error("sqlite error: {0}")]
    Sqlite(String),

    #[error("exec error: {0}")]
    Exec(String),

    #[error("ipc error: {0}")]
    Ipc(String),

    #[error("mcp error: {0}")]
    Mcp(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("{0}")]
    Other(String),
}

impl From<rusqlite::Error> for ArshyError {
    fn from(e: rusqlite::Error) -> Self {
        ArshyError::Sqlite(e.to_string())
    }
}

impl From<serde_json::Error> for ArshyError {
    fn from(e: serde_json::Error) -> Self {
        ArshyError::Serialization(e.to_string())
    }
}

impl From<toml::de::Error> for ArshyError {
    fn from(e: toml::de::Error) -> Self {
        ArshyError::Config(e.to_string())
    }
}

impl From<toml::ser::Error> for ArshyError {
    fn from(e: toml::ser::Error) -> Self {
        ArshyError::Config(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ArshyError>;
