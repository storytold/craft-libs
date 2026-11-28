#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! MCP server initialization and the envelopes of tools, resources, prompts and completion,
//! independent of transport and of what the server serves. Sessions answer 2025-03-26 by default
//! and can negotiate any of `SUPPORTED_PROTOCOL_VERSIONS`.
//! The caller owns routing, transport and JSON-RPC batching. Compatibility mode preserves servers
//! that accepted calls before initialization and initialize in a batch; strict mode enforces the
//! initialization ordering and batch prohibition. Create one session per connection.

use std::collections::BTreeMap;

use craft_jsonrpc::{Limits, Request, RpcError};
use serde_json::{Value, json};

pub mod roots;

/// The version a session answers with unless `Session::with_versions` sets others.
pub const PROTOCOL_VERSION: &str = "2025-03-26";

/// Protocol versions whose initialization this crate implements, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// Versions that removed JSON-RPC batching.
const BATCHLESS_VERSIONS: [&str; 1] = ["2025-06-18"];

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
    negotiated: Option<String>,
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
            negotiated: None,
            initialized: false,
        }
    }

    /// Set the protocol versions the session accepts, in order of preference. A client that asks
    /// for one of them gets it back; any other request gets the first. Each version must be in
    /// `SUPPORTED_PROTOCOL_VERSIONS`, and the list must be non-empty and free of repeats.
    pub fn with_versions(mut self, versions: &[&str]) -> Result<Self, RpcError> {
        if versions.is_empty() {
            return Err(RpcError::new(-32602, "Protocol versions must not be empty"));
        }
        for (i, version) in versions.iter().enumerate() {
            if !SUPPORTED_PROTOCOL_VERSIONS.contains(version) {
                return Err(RpcError::new(-32602, format!("Unsupported protocol version {version}")));
            }
            if versions[..i].contains(version) {
                return Err(RpcError::new(-32602, format!("Repeated protocol version {version}")));
            }
        }
        self.versions = versions.iter().map(|v| (*v).to_owned()).collect();
        Ok(self)
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

    /// The protocol version the last successful initialize answered with.
    pub fn negotiated_version(&self) -> Option<&str> {
        self.negotiated.as_deref()
    }

    fn answer_version(&self, asked: Option<&str>) -> String {
        asked
            .filter(|v| self.versions.iter().any(|supported| supported == v))
            .unwrap_or_else(|| self.versions.first().map(String::as_str).unwrap_or(PROTOCOL_VERSION))
            .to_owned()
    }

    /// Whether the caller may dispatch a JSON-RPC batch. False in strict mode once a version
    /// without batching (2025-06-18) is negotiated; reject such a batch envelope with
    /// -32600 before dispatching it, since `notification` cannot tell a batched notification apart.
    pub fn accepts_batches(&self) -> bool {
        self.policy != Policy::Strict || !self.negotiated.as_deref().is_some_and(|v| BATCHLESS_VERSIONS.contains(&v))
    }

    /// Handles `initialize` and `ping`. Returns `None` for a request the app should route.
    /// `in_batch` must describe the outer JSON-RPC envelope. A batched request is rejected
    /// when `accepts_batches` is false.
    pub fn request(&mut self, request: Request<'_>, in_batch: bool) -> Option<Result<Value, RpcError>> {
        if in_batch && !self.accepts_batches() {
            return Some(Err(RpcError::new(-32600, "Invalid Request")));
        }
        if request.method == "ping" {
            return Some(Ok(json!({})));
        }
        if request.method == "initialize" {
            if self.policy == Policy::Strict && (in_batch || self.state != State::New) {
                return Some(Err(RpcError::new(-32600, "Invalid Request")));
            }
            if self.permissive_initialize {
                let asked = request.params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str);
                let version = self.answer_version(asked);
                let result = json!({"protocolVersion": version, "capabilities": self.capabilities, "serverInfo": self.server_info});
                self.state = State::AwaitingInitialized;
                self.negotiated = Some(version);
                return Some(Ok(self.with_instructions(result)));
            }
            let Some(params) = request.params.and_then(Value::as_object) else {
                return Some(Err(invalid_initialize()));
            };
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let valid =
                asked.is_some() && params.get("capabilities").is_some_and(Value::is_object) && params.get("clientInfo").is_some_and(Value::is_object);
            let identity = params.get("clientInfo").and_then(Value::as_object);
            let strict_identity = identity
                .is_some_and(|info| info.get("name").and_then(Value::as_str).is_some() && info.get("version").and_then(Value::as_str).is_some());
            if !valid || (self.policy == Policy::Strict && !strict_identity) {
                return Some(Err(invalid_initialize()));
            }
            let version = self.answer_version(asked);
            let result =
                json!({"protocolVersion": version, "capabilities": self.capabilities, "serverInfo": {"name": self.name, "version": self.version}});
            self.state = State::AwaitingInitialized;
            self.negotiated = Some(version);
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

/// Most values one `completion/complete` result may carry.
pub const MAX_COMPLETIONS: usize = 100;

/// One page of a `resources/list`, `resources/templates/list` or `prompts/list` result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub items: Vec<Value>,
    /// The cursor a client sends to get the next page; `None` on the last page.
    pub next_cursor: Option<String>,
}

impl Page {
    /// Every item in one page, for a server that does not paginate.
    pub fn all(items: Vec<Value>) -> Self {
        Self { items, next_cursor: None }
    }
}

/// Take the page of `items` that starts at the `cursor` in a list request's parameters, with at most
/// `page_size` items (0 puts every item in one page). The helper accepts canonical positive
/// decimal offsets divisible by `page_size`. Parameters that are not an object, and malformed
/// cursors, are rejected with -32602. The helper returns an empty last page for a cursor past
/// the list's end, including after the list shrinks.
pub fn page(params: Option<&Value>, mut items: Vec<Value>, page_size: usize) -> Result<Page, RpcError> {
    if params.is_some_and(|p| !p.is_object()) {
        return Err(RpcError::new(-32602, "List parameters must be an object"));
    }
    let start = match params.and_then(|p| p.get("cursor")) {
        None => 0,
        Some(Value::String(cursor)) => match cursor.parse::<usize>() {
            Ok(offset) if page_size > 0 && offset > 0 && offset % page_size == 0 && offset.to_string() == *cursor => offset,
            _ => return Err(invalid_cursor()),
        },
        Some(_) => return Err(invalid_cursor()),
    };
    let start = start.min(items.len());
    items.drain(..start);
    let mut next_cursor = None;
    if page_size > 0 && items.len() > page_size {
        items.truncate(page_size);
        next_cursor = Some((start + page_size).to_string());
    }
    Ok(Page { items, next_cursor })
}

fn invalid_cursor() -> RpcError {
    RpcError::new(-32602, "Invalid cursor")
}

/// Wrap a page of MCP `Resource` objects in the `resources/list` result.
pub fn resource_list(page: Page) -> Value {
    list_result("resources", page)
}

/// Wrap a page of MCP `ResourceTemplate` objects in the `resources/templates/list` result.
pub fn resource_template_list(page: Page) -> Value {
    list_result("resourceTemplates", page)
}

/// Wrap a page of MCP `Prompt` objects in the `prompts/list` result.
pub fn prompt_list(page: Page) -> Value {
    list_result("prompts", page)
}

fn list_result(key: &str, page: Page) -> Value {
    let mut result = serde_json::Map::new();
    result.insert(key.to_owned(), Value::Array(page.items));
    if let Some(cursor) = page.next_cursor {
        result.insert("nextCursor".to_owned(), Value::String(cursor));
    }
    Value::Object(result)
}

/// Validate `resources/read` parameters and return the requested URI.
pub fn resource_read(params: &Value) -> Result<&str, RpcError> {
    params.get("uri").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "Missing resource URI"))
}

/// The error MCP specifies for a `resources/read` of a URI the server does not have.
pub fn resource_not_found(uri: &str) -> RpcError {
    RpcError { code: -32002, message: "Resource not found".into(), data: Some(json!({"uri": uri})) }
}

/// Text contents of a resource, for [`read_result`].
pub fn text_contents(uri: &str, mime_type: Option<&str>, text: &str) -> Value {
    contents(uri, mime_type, "text", text)
}

/// Binary contents of a resource, for [`read_result`]. `base64` is the data in standard base64.
pub fn blob_contents(uri: &str, mime_type: Option<&str>, base64: &str) -> Value {
    contents(uri, mime_type, "blob", base64)
}

fn contents(uri: &str, mime_type: Option<&str>, key: &str, value: &str) -> Value {
    let mut result = serde_json::Map::new();
    result.insert("uri".to_owned(), json!(uri));
    if let Some(mime_type) = mime_type {
        result.insert("mimeType".to_owned(), json!(mime_type));
    }
    result.insert(key.to_owned(), json!(value));
    Value::Object(result)
}

/// Wrap resource contents in the `resources/read` result.
pub fn read_result(contents: Vec<Value>) -> Value {
    json!({"contents": contents})
}

/// Validate `prompts/get` parameters and return the prompt name and its arguments. MCP prompt
/// arguments are strings, so an argument of any other type is rejected with -32602.
pub fn prompt_get(params: &Value) -> Result<(&str, BTreeMap<&str, &str>), RpcError> {
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "Missing prompt name"))?;
    let arguments = match params.get("arguments") {
        None => BTreeMap::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, value)| value.as_str().map(|v| (key.as_str(), v)))
            .collect::<Option<BTreeMap<_, _>>>()
            .ok_or_else(|| RpcError::new(-32602, "Prompt arguments must be strings"))?,
        Some(_) => return Err(RpcError::new(-32602, "Prompt arguments must be an object")),
    };
    Ok((name, arguments))
}

/// Wrap prompt messages (MCP `PromptMessage` objects) in the `prompts/get` result.
pub fn prompt_result(description: Option<&str>, messages: Vec<Value>) -> Value {
    let mut result = json!({"messages": messages});
    if let Some(description) = description {
        result["description"] = json!(description);
    }
    result
}

/// A prompt message with one text block, from `"user"` or `"assistant"`.
pub fn text_message(role: &str, text: &str) -> Value {
    json!({"role": role, "content": {"type": "text", "text": text}})
}

/// What a `completion/complete` request refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionRef<'a> {
    /// A prompt, by name (`ref/prompt`).
    Prompt(&'a str),
    /// A resource or resource template, by URI or URI template (`ref/resource`).
    Resource(&'a str),
}

/// The parameters of a `completion/complete` request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion<'a> {
    pub reference: CompletionRef<'a>,
    /// The name of the argument being completed.
    pub argument: &'a str,
    /// What the user has typed for it so far.
    pub value: &'a str,
}

/// Validate `completion/complete` parameters.
pub fn completion_request(params: &Value) -> Result<Completion<'_>, RpcError> {
    let invalid = |message: &str| RpcError::new(-32602, message);
    let reference = params.get("ref").ok_or_else(|| invalid("Missing completion reference"))?;
    let field = |key: &str| reference.get(key).and_then(Value::as_str);
    let reference = match reference.get("type").and_then(Value::as_str) {
        Some("ref/prompt") => CompletionRef::Prompt(field("name").ok_or_else(|| invalid("Missing prompt name"))?),
        Some("ref/resource") => CompletionRef::Resource(field("uri").ok_or_else(|| invalid("Missing resource URI"))?),
        _ => return Err(invalid("Unknown completion reference type")),
    };
    let argument = params.get("argument");
    let text = |key: &str| argument.and_then(|a| a.get(key)).and_then(Value::as_str);
    let (Some(name), Some(value)) = (text("name"), text("value")) else {
        return Err(invalid("Completion argument needs a name and a value"));
    };
    Ok(Completion { reference, argument: name, value })
}

/// Build the `completion/complete` result from every matching value. The result carries the first
/// [`MAX_COMPLETIONS`] values, the number of matches in `total`, and whether any were left out in
/// `hasMore`.
pub fn completion_result(values: impl IntoIterator<Item = String>) -> Value {
    let mut kept = Vec::new();
    let mut total = 0usize;
    for value in values {
        if kept.len() < MAX_COMPLETIONS {
            kept.push(value);
        }
        total += 1;
    }
    json!({"completion": {"values": kept, "total": total, "hasMore": total > MAX_COMPLETIONS}})
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
            assert_eq!(s.negotiated_version(), Some(expected));
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
    fn validated_initialize_negotiates_configured_versions() {
        let mut s = Session::new("test", "1", Policy::Compatible).with_versions(&SUPPORTED_PROTOCOL_VERSIONS).unwrap();
        assert_eq!(s.negotiated_version(), None);
        for (asked, expected) in [("2024-11-05", "2024-11-05"), ("2025-03-26", "2025-03-26"), ("2025-06-18", "2025-06-18"), ("1.0.0", "2025-06-18")] {
            let p = json!({"protocolVersion": asked, "capabilities": {}, "clientInfo": {}});
            let r = s.request(call("initialize", Some(&p)), false).unwrap().unwrap();
            assert_eq!(r["protocolVersion"], expected);
            assert_eq!(s.negotiated_version(), Some(expected));
        }
        let mut old_first = Session::new("test", "1", Policy::Compatible).with_versions(&["2025-03-26", "2025-06-18"]).unwrap();
        let p = json!({"protocolVersion": "2099-01-01", "capabilities": {}, "clientInfo": {}});
        assert_eq!(old_first.request(call("initialize", Some(&p)), false).unwrap().unwrap()["protocolVersion"], "2025-03-26");
    }

    #[test]
    fn version_lists_reject_empty_unknown_and_repeated_entries() {
        let new = || Session::new("test", "1", Policy::Compatible);
        assert!(new().with_versions(&[]).is_err());
        assert!(new().with_versions(&["2099-01-01"]).is_err());
        assert!(new().with_versions(&["2025-06-18", "2025-06-18"]).is_err());
        let mut s = new();
        let p = json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {}});
        assert!(s.request(call("initialize", Some(&json!({}))), false).unwrap().is_err());
        assert_eq!(s.negotiated_version(), None);
        assert_eq!(s.request(call("initialize", Some(&p)), false).unwrap().unwrap()["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn strict_rejects_batches_after_negotiating_a_version_without_them() {
        let p = |v: &str| json!({"protocolVersion": v, "capabilities": {}, "clientInfo": {"name": "client", "version": "1"}});
        let mut s = Session::new("test", "1", Policy::Strict).with_versions(&SUPPORTED_PROTOCOL_VERSIONS).unwrap();
        assert!(s.accepts_batches());
        assert!(s.request(call("initialize", Some(&p("2025-06-18"))), false).unwrap().is_ok());
        assert!(!s.accepts_batches());
        assert_eq!(s.request(call("ping", None), true).unwrap().unwrap_err().code, -32600);
        assert_eq!(s.request(call("tools/list", None), true).unwrap().unwrap_err().code, -32600);
        assert!(s.request(call("tools/list", None), false).is_none());
        let mut older = Session::new("test", "1", Policy::Strict).with_versions(&SUPPORTED_PROTOCOL_VERSIONS).unwrap();
        assert!(older.request(call("initialize", Some(&p("2025-03-26"))), false).unwrap().is_ok());
        assert!(older.accepts_batches());
        assert!(older.request(call("tools/list", None), true).is_none());
        let mut compatible = Session::new("test", "1", Policy::Compatible).with_versions(&SUPPORTED_PROTOCOL_VERSIONS).unwrap();
        assert!(compatible.request(call("initialize", Some(&p("2025-06-18"))), false).unwrap().is_ok());
        assert!(compatible.accepts_batches());
        assert!(compatible.request(call("tools/list", None), true).is_none());
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

    #[test]
    fn list_pages_follow_their_cursors_to_the_end() {
        let items: Vec<Value> = (0..5).map(|i| json!({"name": i})).collect();
        let mut params = None;
        let mut names = Vec::new();
        let mut pages = 0;
        loop {
            let page = page(params.as_ref(), items.clone(), 2).unwrap();
            let result = prompt_list(page);
            pages += 1;
            names.extend(result["prompts"].as_array().unwrap().iter().map(|p| p["name"].clone()));
            match result.get("nextCursor") {
                Some(cursor) => params = Some(json!({"cursor": cursor})),
                None => break,
            }
        }
        assert_eq!((pages, names), (3, (0..5).map(|i| json!(i)).collect::<Vec<_>>()));
        assert_eq!(resource_list(page(None, items.clone(), 0).unwrap()), json!({"resources": items}));
        let past_the_end = page(Some(&json!({"cursor": "8"})), items, 2).unwrap();
        assert_eq!(resource_template_list(past_the_end), json!({"resourceTemplates": []}));
    }

    #[test]
    fn list_cursors_that_were_not_handed_out_are_invalid_params() {
        for cursor in [json!(""), json!("-1"), json!("+1"), json!("1.0"), json!("abc"), json!(2), json!(null), json!("99999999999999999999999")] {
            let error = page(Some(&json!({"cursor": cursor})), vec![json!(1)], 1).unwrap_err();
            assert_eq!(error.code, -32602, "{cursor}");
        }
        for cursor in ["1", "02", "0"] {
            assert_eq!(page(Some(&json!({"cursor": cursor})), vec![json!(1); 5], 2).unwrap_err().code, -32602, "{cursor}");
        }
        assert_eq!(page(Some(&json!({"cursor": "2"})), vec![json!(1); 5], 0).unwrap_err().code, -32602);
        for params in [json!(null), json!([]), json!("0")] {
            assert_eq!(page(Some(&params), vec![json!(1)], 1).unwrap_err().code, -32602, "{params}");
        }
    }

    #[test]
    fn resource_reads_validate_the_uri_and_build_contents() {
        assert_eq!(resource_read(&json!({"uri": "x://a"})).unwrap(), "x://a");
        assert_eq!(resource_read(&json!({"uri": 1})).unwrap_err().code, -32602);
        let missing = resource_not_found("x://b");
        assert_eq!((missing.code, missing.data), (-32002, Some(json!({"uri": "x://b"}))));
        let result = read_result(vec![text_contents("x://a", Some("text/markdown"), "# A"), blob_contents("x://c", None, "AAE=")]);
        assert_eq!(result, json!({"contents": [{"uri": "x://a", "mimeType": "text/markdown", "text": "# A"}, {"uri": "x://c", "blob": "AAE="}]}));
    }

    #[test]
    fn prompt_arguments_must_be_strings() {
        let params = json!({"name": "describe", "arguments": {"part": "bracket"}});
        let (name, args) = prompt_get(&params).unwrap();
        assert_eq!((name, args.get("part").copied()), ("describe", Some("bracket")));
        assert!(prompt_get(&json!({"name": "describe"})).unwrap().1.is_empty());
        for params in [json!({}), json!({"name": "d", "arguments": {"n": 1}}), json!({"name": "d", "arguments": ["x"]})] {
            assert_eq!(prompt_get(&params).unwrap_err().code, -32602, "{params}");
        }
        let result = prompt_result(Some("Describe a part"), vec![text_message("user", "Describe bracket")]);
        assert_eq!(result["messages"][0]["content"], json!({"type": "text", "text": "Describe bracket"}));
        assert_eq!(result["description"], "Describe a part");
    }

    #[test]
    fn completion_requests_name_a_prompt_or_a_resource() {
        let prompt = json!({"ref": {"type": "ref/prompt", "name": "describe"}, "argument": {"name": "part", "value": "br"}});
        assert_eq!(completion_request(&prompt).unwrap(), Completion { reference: CompletionRef::Prompt("describe"), argument: "part", value: "br" });
        let resource = json!({"ref": {"type": "ref/resource", "uri": "lib://parts/{part}"}, "argument": {"name": "part", "value": ""}});
        assert_eq!(completion_request(&resource).unwrap().reference, CompletionRef::Resource("lib://parts/{part}"));
        for params in [
            json!({"ref": {"type": "ref/tool", "name": "x"}, "argument": {"name": "a", "value": ""}}),
            json!({"ref": {"type": "ref/prompt"}, "argument": {"name": "a", "value": ""}}),
            json!({"ref": {"type": "ref/prompt", "name": "x"}, "argument": {"name": "a"}}),
            json!({"argument": {"name": "a", "value": ""}}),
        ] {
            assert_eq!(completion_request(&params).unwrap_err().code, -32602, "{params}");
        }
    }

    #[test]
    fn completion_results_carry_at_most_a_hundred_values() {
        let result = completion_result((0..150).map(|i| i.to_string()));
        assert_eq!(result["completion"]["values"].as_array().map(Vec::len), Some(MAX_COMPLETIONS));
        assert_eq!((&result["completion"]["total"], &result["completion"]["hasMore"]), (&json!(150), &json!(true)));
        assert_eq!(completion_result(["a".to_owned()]), json!({"completion": {"values": ["a"], "total": 1, "hasMore": false}}));
    }
}
