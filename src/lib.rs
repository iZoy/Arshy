//! Arshy — AI Agent native shell execution layer.
//!
//! Arshy is an MCP server that provides structured, event-driven shell execution
//! for AI agents via MCP protocol over stdio.

pub mod config;
pub mod daemon;
pub mod error;
pub mod ipc;
pub mod mcp;

pub use error::{ArshyError, Result};
