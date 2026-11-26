#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! MCP 2025-03-26 server initialization and tool envelopes, independent of transport and tools.
//! The caller owns routing, transport and JSON-RPC batching. Compatibility mode preserves servers
//! that accepted calls before initialization and initialize in a batch; strict mode enforces the
//! initialization ordering and batch prohibition. Create one session per connection.

use craft_jsonrpc::{Limits, Request, RpcError};
use serde_json::{Value, json};

pub mod roots;

pub const PROTOCOL_VERSION: &str = "2025-03-26";

/// JSON-RPC limits for an MCP connection. MCP requires a string or integer request ID, so
/// `craft_jsonrpc::process` answers a null or fractional ID with an invalid-request error and does
/// not call the request handler. The size bounds are `Limits::default()`'s; override them with
/// struct update syntax, as in `Limits { max_message_bytes, ..jsonrpc_limits() }`.
pub fn jsonrpc_limits() -> Limits {
    Limits { integer_ids_only: true, allow_null_ids: false, ..Limits::default() }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// Preserve existing permissive servers, including minimal client information.
    #[default]
    Compatible,
    /// Enforce the initialize request and client identity required by MCP.
    Strict,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    New,
    AwaitingInitialized,
    Ready,
}

/// State for one connection. Unknown requests are left to the caller's router.
pub struct Session {
    name: String,
    version: String,
    policy: Policy,
    state: State,
    capabilities: Value,
    versions: Vec<String>,
    permissive_initialize: bool,
    server_info: Option<Value>,
    instructions: Option<String>,
    initialized: bool,
}

impl Session {
    pub fn new(name: impl Into<String>, version: impl Into<String>, policy: Policy) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            policy,
            state: State::New,
            capabilities: json!({"tools": {}}),
            versions: vec![PROTOCOL_VERSION.into()],
            permissive_initialize: false,
            server_info: None,
            instructions: None,
            initialized: false,
        }
    }

    /// Set the server capabilities returned at initialization. The value must be an object.
    pub fn with_capabilities(mut self, capabilities: Value) -> Result<Self, RpcError> {
        if !capabilities.is_object() {
            return Err(RpcError::new(-32602, "Capabilities must be an object"));
        }
        self.capabilities = capabilities;
        Ok(self)
    }

    /// Configure a permissive legacy server: absent or malformed initialize parameters are
    /// accepted, supported client versions are echoed, and unknown versions use the first entry.
    pub fn with_legacy_initialize(mut self, versions: &[&str], server_info: Value) -> Self {
        if !versions.is_empty() {
            self.versions = versions.iter().map(|v| (*v).to_owned()).collect();
        }
        self.permissive_initialize = true;
        self.server_info = Some(server_info);
        self
    }

    /// Update connection-specific instructions returned by subsequent initialize requests.
    pub fn set_instructions(&mut self, instructions: String) {
        self.instructions = Some(instructions);
    }

    /// Whether an initialized notification has been observed, including before initialize.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Handles `initialize` and `ping`. Returns `None` for a request the app should route.
    /// `in_batch` must describe the outer JSON-RPC envelope.
    pub fn request(&mut self, request: Request<'_>, in_batch: bool) -> Option<Result<Value, RpcError>> {
        if request.method == "ping" {
            return Some(Ok(json!({})));
        }
        if request.method == "initialize" {
            if self.policy == Policy::Strict && (in_batch || self.state != State::New) {
                return Some(Err(RpcError::new(-32600, "Invalid Request")));
            }
            if self.permissive_initialize {
                let asked = request.params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str);
                let version = asked
                    .filter(|v| self.versions.iter().any(|supported| supported == v))
                    .unwrap_or_else(|| self.versions.first().map(String::as_str).unwrap_or(PROTOCOL_VERSION));
                let result = json!({"protocolVersion": version, "capabilities": self.capabilities, "serverInfo": self.server_info});
                self.state = State::AwaitingInitialized;
                return Some(Ok(self.with_instructions(result)));
            }
            let Some(params) = request.params.and_then(Value::as_object) else {
                return Some(Err(invalid_initialize()));
            };
            let valid = params.get("protocolVersion").and_then(Value::as_str).is_some()
                && params.get("capabilities").is_some_and(Value::is_object)
                && params.get("clientInfo").is_some_and(Value::is_object);
            let identity = params.get("clientInfo").and_then(Value::as_object);
            let strict_identity = identity
                .is_some_and(|info| info.get("name").and_then(Value::as_str).is_some() && info.get("version").and_then(Value::as_str).is_some());
            if !valid || (self.policy == Policy::Strict && !strict_identity) {
                return Some(Err(invalid_initialize()));
            }
            self.state = State::AwaitingInitialized;
            let result = json!({"protocolVersion": PROTOCOL_VERSION, "capabilities": self.capabilities, "serverInfo": {"name": self.name, "version": self.version}});
            return Some(Ok(self.with_instructions(result)));
        }
        if self.policy == Policy::Strict && self.state == State::New {
            return Some(Err(RpcError::new(-32600, "Server not initialized")));
        }
        None
    }

    fn with_instructions(&self, mut result: Value) -> Value {
        if let Some(instructions) = &self.instructions {
            result["instructions"] = json!(instructions);
        }
        result
    }

    /// Observe a JSON-RPC notification; notifications never receive replies.
    pub fn notification(&mut self, request: Request<'_>) {
        if request.method == "notifications/initialized" {
            self.initialized = true;
            if self.state == State::AwaitingInitialized {
                self.state = State::Ready;
            }
        }
    }
}

fn invalid_initialize() -> RpcError {
    RpcError::new(-32602, "Invalid initialize parameters")
}

/// Validate MCP tool-call parameters without owning the application's tool routing.
pub fn tool_call(params: &Value) -> Result<(&str, Value), RpcError> {
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "Missing tool name"))?;
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err(RpcError::new(-32602, "Tool arguments must be an object"));
    }
    Ok((name, arguments))
}

/// Wrap an application-provided tool list in the MCP result envelope.
pub fn tool_list(tools: Value) -> Value {
    json!({"tools": tools})
}

/// Construct the text result currently used by Craft's MCP tools.
pub fn text_tool_result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call<'a>(method: &'a str, params: Option<&'a Value>) -> Request<'a> {
        Request { id: None, method, params }
    }

    #[test]
    fn legacy_initialization_negotiates_and_accepts_missing_parameters() {
        let mut s = Session::new("test", "1", Policy::Compatible).with_legacy_initialize(&["new", "old"], json!({"name":"custom", "title":"Title"}));
        s.notification(call("notifications/initialized", None));
        assert!(s.is_initialized());
        s.set_instructions("help".into());
        for (params, expected) in
            [(None, "new"), (Some(json!({"protocolVersion":"old"})), "old"), (Some(json!({"protocolVersion":"unknown"})), "new")]
        {
            let result = s.request(call("initialize", params.as_ref()), false).unwrap().unwrap();
            assert_eq!(result["protocolVersion"], expected);
            assert_eq!(result["serverInfo"]["title"], "Title");
            assert_eq!(result["instructions"], "help");
        }
    }

    #[test]
    fn compatible_negotiation_preserves_existing_clients() {
        let mut s = Session::new("test", "1", Policy::Compatible);
        assert!(s.request(call("tools/list", None), false).is_none());
        let p = json!({"protocolVersion":"old", "capabilities":{}, "clientInfo":{}});
        let r = s.request(call("initialize", Some(&p)), true).unwrap().unwrap();
        assert_eq!(r["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(r["serverInfo"]["name"], "test");
        assert_eq!(s.state(), State::AwaitingInitialized);
        s.notification(call("notifications/initialized", None));
        assert_eq!(s.state(), State::Ready);
    }

    #[test]
    fn strict_rejects_batch_and_calls_before_initialization() {
        let mut s = Session::new("test", "1", Policy::Strict);
        assert_eq!(s.request(call("tools/list", None), false).unwrap().unwrap_err().code, -32600);
        assert!(s.request(call("ping", None), false).unwrap().is_ok());
        let p = json!({"protocolVersion":PROTOCOL_VERSION, "capabilities":{}, "clientInfo":{"name":"client", "version":"1"}});
        assert!(s.request(call("initialize", Some(&p)), true).unwrap().is_err());
        assert_eq!(s.state(), State::New);
        assert!(s.request(call("initialize", Some(&p)), false).unwrap().is_ok());
        assert!(s.request(call("initialize", Some(&p)), false).unwrap().is_err());
        s.notification(call("notifications/initialized", None));
        assert_eq!(s.state(), State::Ready);
    }

    #[test]
    fn capabilities_are_configurable_and_must_be_an_object() {
        assert!(Session::new("test", "1", Policy::Compatible).with_capabilities(json!([])).is_err());
        let mut session = Session::new("test", "1", Policy::Compatible).with_capabilities(json!({"resources": {"subscribe": true}})).unwrap();
        let params = json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {}});
        let result = session.request(call("initialize", Some(&params)), false).unwrap().unwrap();
        assert_eq!(result["capabilities"], json!({"resources": {"subscribe": true}}));
    }

    #[test]
    fn validated_initialization_returns_instructions() {
        let p = json!({"protocolVersion":PROTOCOL_VERSION, "capabilities":{}, "clientInfo":{"name":"client", "version":"1"}});
        for policy in [Policy::Compatible, Policy::Strict] {
            let mut s = Session::new("test", "1", policy);
            assert!(s.request(call("initialize", Some(&p)), false).unwrap().unwrap().get("instructions").is_none());
            let mut s = Session::new("test", "1", policy);
            s.set_instructions("help".into());
            let result = s.request(call("initialize", Some(&p)), false).unwrap().unwrap();
            assert_eq!(result["instructions"], "help");
        }
    }

    #[test]
    fn malformed_initialize_does_not_change_state() {
        let mut s = Session::new("test", "1", Policy::Strict);
        for p in [json!({}), json!({"protocolVersion":"x", "capabilities":{}, "clientInfo":{}})] {
            assert_eq!(s.request(call("initialize", Some(&p)), false).unwrap().unwrap_err().code, -32602);
            assert_eq!(s.state(), State::New);
        }
    }

    #[test]
    fn tool_envelopes_validate_arguments_and_preserve_results() {
        let p = json!({"name":"open"});
        assert_eq!(tool_call(&p).unwrap(), ("open", json!({})));
        assert_eq!(tool_call(&json!({"name":"open", "arguments":[]})).unwrap_err().code, -32602);
        assert_eq!(tool_call(&json!({})).unwrap_err().code, -32602);
        assert_eq!(text_tool_result("bad", true), json!({"content":[{"type":"text", "text":"bad"}], "isError":true}));
    }

    #[test]
    fn jsonrpc_limits_reject_ids_mcp_forbids() {
        let called = std::cell::Cell::new(0);
        let reply = |id: Value| {
            let input = json!({"jsonrpc":"2.0", "id":id, "method":"ping"});
            let handler = |_: Request<'_>| {
                called.set(called.get() + 1);
                Ok(json!({}))
            };
            craft_jsonrpc::process(&input, jsonrpc_limits(), handler, |_| {}).unwrap().unwrap()
        };
        for id in [json!(null), json!(1.5)] {
            let r = reply(id);
            assert_eq!(r["error"]["code"], -32600);
            assert_eq!(called.get(), 0);
        }
        for id in [json!(7), json!("seven")] {
            assert_eq!(reply(id.clone())["id"], id);
        }
        assert_eq!(called.get(), 2);
    }

    #[test]
    fn jsonrpc_batch_preserves_ids_and_skips_notifications() {
        use std::cell::RefCell;
        let session = RefCell::new(Session::new("test", "1", Policy::Compatible));
        let input = json!([
            {"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{"protocolVersion":"2025-03-26", "capabilities":{}, "clientInfo":{}}},
            {"jsonrpc":"2.0", "method":"notifications/initialized"},
            {"jsonrpc":"2.0", "id":"tools", "method":"tools/list"}
        ]);
        let reply = craft_jsonrpc::process(
            &input,
            craft_jsonrpc::Limits::default(),
            |r| session.borrow_mut().request(r, true).unwrap_or_else(|| Ok(tool_list(json!([])))),
            |r| session.borrow_mut().notification(r),
        )
        .unwrap()
        .unwrap();
        assert_eq!(reply.as_array().unwrap().len(), 2);
        assert_eq!(reply[0]["id"], 1);
        assert_eq!(reply[1]["id"], "tools");
        assert_eq!(reply[1]["result"]["tools"], json!([]));
        assert_eq!(session.borrow().state(), State::Ready);
    }
}
