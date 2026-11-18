//! # craft-agent-bridge
//!
//! Native Model Context Protocol (MCP) server, Hermes Agent skill provider, and fast JSON-RPC
//! IPC bridge for the Storytold ArtCraft Suite (WordCraft, PdfCraft, PhotoCraft, CADCraft, etc.).
//!
//! Rather than forcing AI agents to execute slow, brittle Python wrappers or simulate mouse clicks,
//! `craft-agent-bridge` allows any agent (Hermes, Claude, Antigravity, or custom local models)
//! to connect directly over standard stdio JSON-RPC or Unix domain socket.
//!
//! Provides:
//! - Direct, sub-millisecond document/canvas operations
//! - Native Tool definitions conforming to MCP 2024-11-05
//! - Auto-generated Hermes Agent Skill markdown (`SKILL.md`) for any app in the suite
//! - Safe state inspection, rollback, and execution without graphical lag

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Standard JSON-RPC 2.0 Request received from an AI Agent or MCP Client.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: Option<serde_json::Value>,
}

/// Standard JSON-RPC 2.0 Response returned to the AI Agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Description of an MCP Tool exposed natively by a Craft App.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Core Bridge Trait implemented by any Craft App engine or document model.
pub trait AppActionHandler: Send + Sync {
    /// Returns the app's unique identifier (e.g. "wordcraft", "photocraft", "pdfcraft").
    fn app_id(&self) -> &str;

    /// Lists all tools/actions this app exposes natively to AI agents.
    fn list_tools(&self) -> Vec<McpTool>;

    /// Executes an agent action natively in memory and returns structured results.
    fn execute_action(&self, tool_name: &str, arguments: serde_json::Value) -> Result<serde_json::Value, String>;
}

/// Native Agent Server that runs over stdio (MCP standard) or socket.
pub struct AgentBridgeServer {
    app_id: String,
    tools: HashMap<String, McpTool>,
    handler: Box<dyn AppActionHandler>,
}

impl AgentBridgeServer {
    /// Creates a new native server for an app.
    pub fn new(handler: Box<dyn AppActionHandler>) -> Self {
        let app_id = handler.app_id().to_string();
        let tools = handler.list_tools().into_iter().map(|t| (t.name.clone(), t)).collect();
        Self { app_id, tools, handler }
    }

    /// Runs a blocking JSON-RPC 2.0 loop over buffered reader and writer streams (e.g. stdio).
    pub fn serve_stdio<R: std::io::BufRead, W: std::io::Write>(
        &self,
        reader: R,
        mut writer: W,
    ) -> std::io::Result<()> {
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_rpc_message(trimmed) {
                writeln!(writer, "{reply}")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    fn validate_tool_call(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Option<JsonRpcError> {
        let Some(tool) = self.tools.get(tool_name) else {
            return Some(JsonRpcError {
                code: -32602,
                message: format!("Unknown tool: {tool_name}"),
                data: None,
            });
        };
        if let Some(required) = tool.input_schema.get("required").and_then(|v| v.as_array()) {
            let obj = arguments.as_object();
            for key in required.iter().filter_map(|v| v.as_str()) {
                let present = obj
                    .and_then(|map| map.get(key))
                    .is_some_and(|v| !v.is_null());
                if !present {
                    return Some(JsonRpcError {
                        code: -32602,
                        message: format!(
                            "Missing required argument '{key}' for tool '{tool_name}'"
                        ),
                        data: None,
                    });
                }
            }
        }
        None
    }

    /// Handles a single incoming line from an AI agent over stdio / JSON-RPC.
    pub fn handle_rpc_message(&self, input: &str) -> Option<String> {
        let req: JsonRpcRequest = match serde_json::from_str(input) {
            Ok(r) => r,
            Err(e) => {
                let err_resp = JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: None,
                    result: None,
                    error: Some(JsonRpcError { code: -32700, message: format!("Parse error: {e}"), data: None }),
                };
                return serde_json::to_string(&err_resp).ok();
            }
        };

        // JSON-RPC 2.0 notifications omit `id` and must not receive a response.
        req.id.as_ref()?;

        let response = match req.method.as_str() {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": format!("craft-mcp-{}", self.app_id),
                        "version": env!("CARGO_PKG_VERSION")
                    }
                })),
                error: None,
            },
            "ping" => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: Some(serde_json::json!({})),
                error: None,
            },
            "tools/list" => {
                let tool_list: Vec<&McpTool> = self.tools.values().collect();
                JsonRpcResponse { jsonrpc: "2.0".into(), id: req.id, result: Some(serde_json::json!({ "tools": tool_list })), error: None }
            }
            "tools/call" => {
                let params = req.params.unwrap_or(serde_json::Value::Null);
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(serde_json::Value::Null);

                if let Some(error) = self.validate_tool_call(tool_name, &arguments) {
                    let response = JsonRpcResponse { jsonrpc: "2.0".into(), id: req.id, result: None, error: Some(error) };
                    return serde_json::to_string(&response).ok();
                }

                match self.handler.execute_action(tool_name, arguments) {
                    Ok(val) => JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id: req.id,
                        result: Some(serde_json::json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": serde_json::to_string_pretty(&val).unwrap_or_default()
                                }
                            ]
                        })),
                        error: None,
                    },
                    Err(err_msg) => JsonRpcResponse {
                        jsonrpc: "2.0".into(),
                        id: req.id,
                        result: Some(serde_json::json!({
                            "content": [
                                { "type": "text", "text": format!("Execution error: {err_msg}") }
                            ],
                            "isError": true
                        })),
                        error: None,
                    },
                }
            }
            _ => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: None,
                error: Some(JsonRpcError { code: -32601, message: format!("Method not found: {}", req.method), data: None }),
            },
        };

        serde_json::to_string(&response).ok()
    }

    /// Generates a complete Hermes Agent Skill file (`SKILL.md`) for this app.
    /// Generates a complete Hermes Agent Skill file (`SKILL.md`) for this app.
    pub fn generate_hermes_skill(&self) -> String {
        let app = &self.app_id;
        let mut out = String::new();
        out.push_str(
            "---
",
        );
        out.push_str(&format!(
            "name: {app}
"
        ));
        out.push_str(&format!(
            "description: \"Native MCP and JSON-RPC control for {app}\"
"
        ));
        out.push_str(
            "---

",
        );
        out.push_str(&format!(
            "# {app} Agent Skill

"
        ));
        out.push_str(
            "Direct native tools supported:

",
        );
        for tool in self.tools.values() {
            out.push_str(&format!(
                "### `{}`
{}

",
                tool.name, tool.description
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockWordCraft;
    impl AppActionHandler for MockWordCraft {
        fn app_id(&self) -> &str {
            "wordcraft"
        }
        fn list_tools(&self) -> Vec<McpTool> {
            vec![McpTool {
                name: "insert_heading".into(),
                description: "Inserts a styled heading into the active document".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "level": { "type": "integer" },
                        "text": { "type": "string" }
                    },
                    "required": ["text"]
                }),
            }]
        }
        fn execute_action(&self, tool_name: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
            if tool_name == "insert_heading" {
                let text = args.get("text").and_then(|t| t.as_str()).unwrap_or("");
                Ok(serde_json::json!({ "status": "success", "inserted": text }))
            } else {
                Err(format!("Unknown tool: {tool_name}"))
            }
        }
    }

    #[test]
    fn rpc_initialize_and_tool_call() {
        let server = AgentBridgeServer::new(Box::new(MockWordCraft));
        let init_resp = server.handle_rpc_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap();
        assert!(init_resp.contains("craft-mcp-wordcraft"));

        let list_resp = server.handle_rpc_message(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#).unwrap();
        assert!(list_resp.contains("insert_heading"));

        let call_resp = server
            .handle_rpc_message(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"insert_heading","arguments":{"text":"מבוא"}}}"#)
            .unwrap();
        assert!(call_resp.contains("מבוא"));
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use std::io::Cursor;

    struct EchoHandler;

    impl AppActionHandler for EchoHandler {
        fn app_id(&self) -> &str {
            "testcraft"
        }

        fn list_tools(&self) -> Vec<McpTool> {
            vec![McpTool {
                name: "open_doc".into(),
                description: "Open a document by path".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }),
            }]
        }

        fn execute_action(
            &self,
            tool_name: &str,
            arguments: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({ "tool": tool_name, "arguments": arguments }))
        }
    }

    #[test]
    fn mcp_tool_serializes_camel_case_input_schema() {
        let server = AgentBridgeServer::new(Box::new(EchoHandler));
        let raw = server
            .handle_rpc_message(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .expect("tools/list response");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        let tool = &parsed["result"]["tools"][0];
        assert!(tool.get("inputSchema").is_some(), "expected inputSchema key in {tool}");
        assert!(tool.get("input_schema").is_none(), "unexpected snake_case input_schema key");
    }

    #[test]
    fn notifications_return_none_and_ping_succeeds() {
        let server = AgentBridgeServer::new(Box::new(EchoHandler));
        assert!(
            server
                .handle_rpc_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none(),
            "notifications without id must not emit a response"
        );

        let raw = server
            .handle_rpc_message(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#)
            .expect("ping response");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["result"], serde_json::json!({}));
        assert!(parsed.get("error").is_none() || parsed["error"].is_null());
    }

    #[test]
    fn tools_call_validates_tool_name_and_required_arguments() {
        let server = AgentBridgeServer::new(Box::new(EchoHandler));

        let unknown: serde_json::Value = serde_json::from_str(
            &server
                .handle_rpc_message(
                    r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"missing_tool","arguments":{}}}"#,
                )
                .expect("unknown tool response"),
        )
        .expect("json");
        assert_eq!(unknown["error"]["code"], -32602);

        let missing_req: serde_json::Value = serde_json::from_str(
            &server
                .handle_rpc_message(
                    r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"open_doc","arguments":{}}}"#,
                )
                .expect("missing required arg response"),
        )
        .expect("json");
        assert_eq!(missing_req["error"]["code"], -32602);

        let valid: serde_json::Value = serde_json::from_str(
            &server
                .handle_rpc_message(
                    r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"open_doc","arguments":{"path":"doc.docx"}}}"#,
                )
                .expect("valid tool call response"),
        )
        .expect("json");
        let valid_text = valid["result"]["content"][0]["text"].as_str().expect("text payload");
        assert!(valid_text.contains("open_doc"), "unexpected payload: {valid_text}");
    }

    #[test]
    fn serve_stdio_processes_lines_and_skips_notifications() {
        let server = AgentBridgeServer::new(Box::new(EchoHandler));
        let input = concat!(
            "\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"ping\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\"params\":{\"name\":\"open_doc\",\"arguments\":{\"path\":\"a.docx\"}}}\n"
        );
        let mut out = Vec::new();
        server
            .serve_stdio(Cursor::new(input.as_bytes()), &mut out)
            .expect("serve_stdio");
        let output_str = String::from_utf8(out).expect("utf8");
        let lines: Vec<&str> = output_str.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("line 0 json");
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("line 1 json");
        assert_eq!(first["id"], 10);
        assert_eq!(second["id"], 11);
        let second_text = second["result"]["content"][0]["text"].as_str().expect("text payload");
        assert!(second_text.contains("a.docx"), "unexpected payload: {second_text}");
    }
}
