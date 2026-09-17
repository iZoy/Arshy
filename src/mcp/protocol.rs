//! MCP JSON-RPC 2.0 message types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Initialize ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
    #[serde(default)]
    pub capabilities: ClientCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<HashMap<String, serde_json::Value>>,
}

// ── Tools ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
    #[serde(rename = "outputSchema", skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallParams {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResponse {
    pub content: Vec<ContentItem>,
    #[serde(rename = "structuredContent", default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentItem {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "resource")]
    Resource { resource: serde_json::Value },
}

// ── Resources ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDefinition {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub text: String,
}

// ── Notification ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
}

/// Log levels for MCP `notifications/message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_item_text_serializes_with_type_tag() {
        let item = ContentItem::Text { text: "hello".into() };
        let s = serde_json::to_string(&item).unwrap();
        assert_eq!(s, r#"{"type":"text","text":"hello"}"#);
        let back: ContentItem = serde_json::from_str(&s).unwrap();
        match back {
            ContentItem::Text { text } => assert_eq!(text, "hello"),
            ContentItem::Resource { .. } => unreachable!("serialized a Text variant"),
        }
    }

    #[test]
    fn tool_call_response_round_trips_structured_content() {
        let response = ToolCallResponse {
            content: vec![ContentItem::Text { text: "ok".into() }],
            structured_content: Some(json!({"status": "completed", "exit_code": 0})),
            is_error: None,
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["structuredContent"]["exit_code"], 0);
        let back: ToolCallResponse = serde_json::from_value(value).unwrap();
        assert_eq!(back.structured_content.unwrap()["status"], "completed");
    }

    #[test]
    fn log_level_uses_lowercase_rename() {
        let cases = [
            (LogLevel::Debug, "\"debug\""),
            (LogLevel::Info, "\"info\""),
            (LogLevel::Warning, "\"warning\""),
            (LogLevel::Error, "\"error\""),
        ];
        for (lvl, expect) in cases {
            assert_eq!(serde_json::to_string(&lvl).unwrap(), expect);
        }
    }

    #[test]
    fn initialize_params_round_trips() {
        let p = InitializeParams {
            protocol_version: "2024-11-05".into(),
            client_info: ClientInfo { name: "claude".into(), version: "1.0".into() },
            capabilities: ClientCapabilities::default(),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: InitializeParams = serde_json::from_str(&s).unwrap();
        assert_eq!(back.protocol_version, "2024-11-05");
        assert_eq!(back.client_info.name, "claude");
        assert_eq!(back.client_info.version, "1.0");
    }

    #[test]
    fn tool_definition_round_trips() {
        let td = ToolDefinition {
            name: "arshy_exec".into(),
            description: "run".into(),
            input_schema: json!({"type": "object"}),
            output_schema: Some(json!({"type": "object"})),
        };
        let s = serde_json::to_string(&td).unwrap();
        let back: ToolDefinition = serde_json::from_str(&s).unwrap();
        assert_eq!(back.name, "arshy_exec");
        assert_eq!(back.input_schema, json!({"type": "object"}));
        assert_eq!(back.output_schema, Some(json!({"type": "object"})));
    }

    #[test]
    fn mcp_notification_round_trips() {
        let n = McpNotification {
            jsonrpc: "2.0".into(),
            method: "notifications/message".into(),
            params: json!({"level": "info"}),
        };
        let s = serde_json::to_string(&n).unwrap();
        let back: McpNotification = serde_json::from_str(&s).unwrap();
        assert_eq!(back.method, "notifications/message");
        assert_eq!(back.params, json!({"level": "info"}));
    }
}
