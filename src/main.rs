use clap::Parser;
use std::path::PathBuf;

mod cli;
mod proxy;

#[derive(Parser, Debug)]
#[command(
    name = "arshy",
    version,
    about = "Structured command execution and diagnostics for AI agents over MCP",
    long_about = "Arshy is a local structured command-execution and diagnostics layer for AI agents. It exposes three MCP tools — arshy_exec, arshy_query, and arshy_task — for running commands, querying events, and retrieving actionable build/test diagnostics."
)]
pub struct Cli {
    /// Path to config file. If not set, defaults are used with env var overrides.
    #[arg(long = "config", verbatim_doc_comment)]
    pub config: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error). Overrides config file.
    #[arg(long = "log-level", verbatim_doc_comment)]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[cfg(test)]
mod tests {
    use super::{Cli, CliCommand};
    use clap::{CommandFactory, Parser};

    #[test]
    fn help_describes_the_three_tool_mcp_surface() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("three MCP tools"));
        assert!(help.contains("arshy_exec"));
        assert!(help.contains("arshy_query"));
        assert!(help.contains("arshy_task"));
        assert!(!help.contains("two MCP tools"));
    }

    #[test]
    fn parses_prompt_config_and_rejects_unreleased_client_adapters() {
        let cli = Cli::try_parse_from(["arshy", "mcp", "config", "--format", "prompt"]).unwrap();
        assert!(matches!(cli.command, Some(CliCommand::Mcp { .. })));
        assert!(Cli::try_parse_from(["arshy", "init", "codex"]).is_err());
        assert!(Cli::try_parse_from(["arshy", "uninit", "codex"]).is_err());
    }
}

#[derive(clap::Subcommand, Debug)]
pub enum CliCommand {
    /// Execute a shell command with non-interactive sh -c (null stdin; no PTY)
    Run {
        command: String,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        timeout_ms: Option<u64>,
        #[arg(long)]
        mode: Option<String>,
        /// Output format: pretty (terminal UI), json (raw JSON), auto (default)
        #[arg(long, default_value = "auto")]
        format: String,
        /// Only return error-severity events
        #[arg(long)]
        errors_only: bool,
        /// Purpose label (e.g. `dogfood`) so stats separate test workloads
        /// from real development; untagged tasks count as real.
        #[arg(long)]
        purpose: Option<String>,
    },
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Query events from a task
    Query {
        task_id: String,
        #[arg(long)]
        event_type: Option<String>,
        #[arg(long)]
        severity: Option<String>,
        #[arg(long)]
        code: Option<String>,
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Kill a running task
    Kill { task_id: String },
    /// Tail task output
    Tail {
        task_id: String,
        #[arg(long, default_value = "50")]
        lines: usize,
        #[arg(long, default_value = "event")]
        format: String,
    },
    /// Prune old task history
    Prune {
        #[arg(long)]
        keep: Option<usize>,
        #[arg(long)]
        older_than: Option<String>,
    },
    /// View or modify arshy configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Show daemon status
    Status,
    /// Manage the daemon process
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Show aggregate statistics
    Stats {
        /// Output format: pretty (terminal UI), json (raw JSON)
        #[arg(long, default_value = "pretty")]
        format: String,
    },
    /// Diagnose the local binary, daemon and generic MCP server
    Doctor {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Generate impact analysis report (used internally by dogfood --report)
    #[command(hide = true)]
    Analyze {
        /// Output format: pretty (terminal UI), json (raw JSON), auto (default)
        #[arg(long, default_value = "auto")]
        format: String,
    },
    /// Manage parsers
    Parser {
        #[command(subcommand)]
        action: ParserAction,
    },
    /// Generic MCP server and configuration helpers
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
    /// Copy the current build over the installed arshy/arshyd binaries
    SelfUpdate {
        /// Install directory (default: ~/.local/bin, or the current binary's dir when already installed)
        #[arg(long)]
        dest: Option<PathBuf>,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum McpAction {
    /// Run the stdio MCP server
    Serve,
    /// Print a client-neutral stdio server entry or setup prompt
    Config {
        /// Output format: json (default), command, or prompt
        #[arg(long, default_value = "json")]
        format: String,
    },
}

#[derive(clap::Subcommand, Debug)]
pub enum ConfigAction {
    /// Get a config value by key (e.g. "daemon.log_level")
    Get { key: String },
    /// List all config values
    List,
    /// Show the config file path
    Path,
}

#[derive(clap::Subcommand, Debug)]
pub enum DaemonAction {
    /// Start the daemon (if not already running)
    Start,
    /// Stop the running daemon
    Stop,
    /// Restart the daemon (stop + start)
    Restart,
}

#[derive(clap::Subcommand, Debug)]
pub enum ParserAction {
    /// Reload parsers from disk and show what changed
    Reload,
    /// List all loaded parsers
    List,
    /// Run parser benchmark across all builtin parsers
    Benchmark,
}

fn main() -> arshy_lib::Result<()> {
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(cli::dispatch(cli))
}
