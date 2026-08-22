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

    #[error("exec error: {0}")]
    Exec(String),

    #[error("ipc error: {0}")]
    Ipc(String),

    #[error("mcp error: {0}")]
    Mcp(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("access denied: {0}")]
    AccessDenied(String),

    /// A command was rejected by the security filter (blocked pattern or
    /// whitelist). Distinct from internal errors so agents can react by
    /// choosing a different command instead of retrying.
    #[error("command blocked: {0}")]
    Blocked(String),

    #[error("{0}")]
    Other(String),
}

impl ArshyError {
    /// Map to a JSON-RPC error code.
    pub fn json_rpc_code(&self) -> i64 {
        use crate::ipc::error_code::*;
        match self {
            Self::TaskNotFound(_) => TASK_NOT_FOUND,
            Self::TaskTimeout { .. } => TASK_TIMEOUT,
            Self::AccessDenied(_) => ACCESS_DENIED,
            Self::Blocked(_) => COMMAND_BLOCKED,
            Self::Ipc(msg) if msg.contains("unknown method") || msg.contains("unknown tool") => {
                METHOD_NOT_FOUND
            }
            Self::Ipc(msg) if msg.contains("missing") || msg.contains("invalid") => INVALID_PARAMS,
            Self::Ipc(msg) if msg.contains("rate limit") || msg.contains("too many concurrent") => {
                RATE_LIMITED
            }
            _ => INTERNAL_ERROR,
        }
    }

    /// Whether the operation can be safely retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::TaskTimeout { .. } => true,
            Self::DaemonUnreachable(_) => true,
            Self::Ipc(msg)
                if msg.contains("timed out")
                    || msg.contains("connection closed")
                    || msg.contains("too many concurrent") =>
            {
                true
            }
            Self::Io(_) => true,
            _ => false,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_mapping() {
        assert_eq!(ArshyError::TaskNotFound("x".into()).json_rpc_code(), -32001);
        assert_eq!(ArshyError::TaskTimeout { duration_ms: 5000 }.json_rpc_code(), -32002);
        assert_eq!(ArshyError::AccessDenied("ro".into()).json_rpc_code(), -32003);
        assert_eq!(ArshyError::Blocked("rm -rf /".into()).json_rpc_code(), -32004);
        assert_eq!(ArshyError::Ipc("unknown method: foo".into()).json_rpc_code(), -32601);
        assert_eq!(ArshyError::Ipc("unknown tool: foo".into()).json_rpc_code(), -32601);
        assert_eq!(
            ArshyError::Ipc("too many concurrent tasks: limit 4 reached".into()).json_rpc_code(),
            -32005
        );
        assert_eq!(ArshyError::Ipc("missing task_id".into()).json_rpc_code(), -32602);
        assert_eq!(ArshyError::Config("bad".into()).json_rpc_code(), -32603);
    }

    #[test]
    fn retry_semantics() {
        assert!(ArshyError::TaskTimeout { duration_ms: 1000 }.is_retryable());
        assert!(ArshyError::DaemonUnreachable("down".into()).is_retryable());
        assert!(ArshyError::Ipc("timed out".into()).is_retryable());
        assert!(ArshyError::Ipc("connection closed".into()).is_retryable());
        assert!(ArshyError::Ipc("too many concurrent tasks".into()).is_retryable());
        assert!(ArshyError::Io(std::io::Error::other("x")).is_retryable());

        assert!(!ArshyError::TaskNotFound("x".into()).is_retryable());
        assert!(!ArshyError::AccessDenied("ro".into()).is_retryable());
        assert!(!ArshyError::Blocked("rm -rf /".into()).is_retryable());
        assert!(!ArshyError::Config("bad".into()).is_retryable());
        assert!(!ArshyError::Ipc("unknown method".into()).is_retryable());
    }
}
