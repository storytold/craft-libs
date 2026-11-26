//! Bounded JSON-RPC 2.0 server envelopes without a transport or application router.
//!
//! Parse a bounded line with [`parse_json`], then pass the value to [`process`]. A host may
//! inspect the parsed value first for a legacy protocol. `process` validates envelopes, dispatches
//! requests and notifications separately, and constructs single or batch replies. [`process_with`]
//! does the same with one [`Handler`] for both requests and notifications. The host owns
//! the transport, method semantics and response-size policy.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(feature = "async-lines")]
pub mod async_lines;
pub mod control;

use serde_json::{Value, json};
use std::io::BufRead;

/// Bounds on a single JSON document and on the number of requests in a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_message_bytes: usize,
    pub max_batch_items: usize,
    /// Reject fractional JSON numbers as IDs for protocols that require integer IDs.
    pub integer_ids_only: bool,
    /// Permit an explicit null request ID, as JSON-RPC does; some protocols prohibit it.
    pub allow_null_ids: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_message_bytes: 64 * 1024 * 1024, max_batch_items: 4096, integer_ids_only: false, allow_null_ids: true }
    }
}

/// Failure before dispatch. An over-limit batch is rejected before any callback is invoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    MessageTooLarge,
    InvalidJson,
    BatchTooLarge,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MessageTooLarge => "JSON-RPC message exceeds the byte limit",
            Self::InvalidJson => "invalid JSON",
            Self::BatchTooLarge => "JSON-RPC batch exceeds the item limit",
        })
    }
}
impl std::error::Error for ParseError {}

impl ParseError {
    /// The JSON-RPC error for this failure: `-32700` for input that is not JSON, and `-32600` for
    /// a message or batch over its limit, which the server did not read as a request.
    pub fn rpc_error(self) -> RpcError {
        match self {
            Self::InvalidJson => RpcError::new(-32700, "Parse error"),
            Self::MessageTooLarge | Self::BatchTooLarge => RpcError::new(-32600, self.to_string()),
        }
    }

    /// Build the error response for this failure. The ID is null because no request ID was read.
    pub fn error_response(self) -> Value {
        error_response(Value::Null, self.rpc_error())
    }
}

impl From<ParseError> for RpcError {
    fn from(error: ParseError) -> Self {
        error.rpc_error()
    }
}

/// Error reading a newline-delimited message. After `TooLong`, close or otherwise discard the
/// remaining input line before reading another message.
#[derive(Debug)]
pub enum LineError {
    Io(std::io::Error),
    TooLong,
}

impl std::fmt::Display for LineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "message read failed: {error}"),
            Self::TooLong => f.write_str("message exceeds the byte limit"),
        }
    }
}
impl std::error::Error for LineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::TooLong => None,
        }
    }
}

/// Read at most `max_bytes` before a newline, without allocating for an oversized line. The
/// newline is consumed but omitted from the returned bytes. EOF with no bytes returns `None`;
/// an empty line returns `Some(Vec::new())`. A read interrupted by a signal is retried, as
/// [`BufRead::read_until`] does.
pub fn read_line<R: BufRead>(reader: &mut R, max_bytes: usize) -> Result<Option<Vec<u8>>, LineError> {
    let mut line = Vec::new();
    loop {
        let bytes = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(LineError::Io(error)),
        };
        if bytes.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let newline = bytes.iter().position(|&byte| byte == b'\n');
        let len = newline.unwrap_or(bytes.len());
        if line.len().saturating_add(len) > max_bytes {
            return Err(LineError::TooLong);
        }
        line.extend_from_slice(&bytes[..len]);
        reader.consume(len + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

/// Discard input through the next newline without buffering it, as after [`LineError::TooLong`].
/// Returns `true` when a newline was consumed, so the next read starts a new message, and `false`
/// at EOF. An interrupted read is retried.
pub fn skip_line<R: BufRead>(reader: &mut R) -> std::io::Result<bool> {
    loop {
        let bytes = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if bytes.is_empty() {
            return Ok(false);
        }
        let newline = bytes.iter().position(|&byte| byte == b'\n');
        let len = newline.map_or(bytes.len(), |index| index + 1);
        reader.consume(len);
        if newline.is_some() {
            return Ok(true);
        }
    }
}

/// Parse JSON after checking its byte length. `serde_json` also bounds nesting depth.
pub fn parse_json(bytes: &[u8], limits: Limits) -> Result<Value, ParseError> {
    if bytes.len() > limits.max_message_bytes {
        return Err(ParseError::MessageTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ParseError::InvalidJson)
}

/// A validated request or notification. A present null `id` is distinct from an omitted `id`.
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub id: Option<&'a Value>,
    pub method: &'a str,
    pub params: Option<&'a Value>,
}

/// Inspect a single request envelope. This validates its JSON-RPC version, method and ID;
/// [`process`] additionally validates parameter shape before dispatching it.
pub fn request(value: &Value, limits: Limits) -> Option<Request<'_>> {
    let object = value.as_object()?;
    let id = object.get("id");
    let valid_id = id.is_none_or(|v| {
        (v.is_null() && limits.allow_null_ids) || v.is_string() || if limits.integer_ids_only { v.is_i64() || v.is_u64() } else { v.is_number() }
    });
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !valid_id {
        return None;
    }
    Some(Request { id, method: object.get("method")?.as_str()?, params: object.get("params") })
}

/// A JSON-RPC error returned by a method handler.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
}

/// Build an error response, including optional error data.
pub fn error_response(id: Value, error: RpcError) -> Value {
    let mut detail = json!({"code": error.code, "message": error.message});
    if let (Some(data), Some(object)) = (error.data, detail.as_object_mut()) {
        object.insert("data".into(), data);
    }
    json!({"jsonrpc": "2.0", "id": id, "error": detail})
}

/// Build the standard parse-error response for input that is not JSON.
pub fn parse_error_response() -> Value {
    error_response(Value::Null, RpcError::new(-32700, "Parse error"))
}

fn invalid_request() -> Value {
    error_response(Value::Null, RpcError::new(-32600, "Invalid Request"))
}

/// Receives the requests and notifications of one message for [`process_with`] and
/// [`process_legacy_with`]. Both methods take `&mut self`, so one handler can update the same
/// state for requests and notifications, in message order, without a `RefCell`.
pub trait Handler {
    /// Handle a request with an ID. The result or error becomes its response.
    fn request(&mut self, request: Request<'_>) -> Result<Value, RpcError>;
    /// Handle a notification, including its `params`. A notification never receives a response.
    fn notification(&mut self, request: Request<'_>);
}

impl<H: Handler + ?Sized> Handler for &mut H {
    fn request(&mut self, request: Request<'_>) -> Result<Value, RpcError> {
        (**self).request(request)
    }
    fn notification(&mut self, request: Request<'_>) {
        (**self).notification(request);
    }
}

struct Callbacks<F, N> {
    on_request: F,
    on_notification: N,
}

impl<F, N> Handler for Callbacks<F, N>
where
    F: FnMut(Request<'_>) -> Result<Value, RpcError>,
    N: FnMut(Request<'_>),
{
    fn request(&mut self, request: Request<'_>) -> Result<Value, RpcError> {
        (self.on_request)(request)
    }
    fn notification(&mut self, request: Request<'_>) {
        (self.on_notification)(request);
    }
}

fn one<H: Handler>(value: &Value, limits: Limits, handler: &mut H) -> Option<Value> {
    let Some(request) = request(value, limits) else { return Some(invalid_request()) };
    let id = request.id;
    if request.params.is_some_and(|v| !v.is_object() && !v.is_array()) {
        return id.map(|id| error_response(id.clone(), RpcError::new(-32602, "Invalid params")));
    }
    let Some(id) = id else {
        handler.notification(request);
        return None;
    };
    Some(match handler.request(request) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => error_response(id.clone(), error),
    })
}

/// Process a parsed JSON-RPC message. Notifications never receive a response. Invalid batch
/// entries produce individual errors; an empty batch produces one invalid-request response.
/// A batch over the limit returns [`ParseError::BatchTooLarge`] without invoking either callback.
pub fn process<F, N>(value: &Value, limits: Limits, on_request: F, on_notification: N) -> Result<Option<Value>, ParseError>
where
    F: FnMut(Request<'_>) -> Result<Value, RpcError>,
    N: FnMut(Request<'_>),
{
    process_with(value, limits, Callbacks { on_request, on_notification })
}

/// Process a parsed JSON-RPC message as [`process`] does, dispatching requests and
/// notifications to one [`Handler`]. A batch over the limit returns
/// [`ParseError::BatchTooLarge`] without calling the handler.
pub fn process_with<H: Handler>(value: &Value, limits: Limits, mut handler: H) -> Result<Option<Value>, ParseError> {
    let Some(batch) = value.as_array() else { return Ok(one(value, limits, &mut handler)) };
    if batch.len() > limits.max_batch_items {
        return Err(ParseError::BatchTooLarge);
    }
    if batch.is_empty() {
        return Ok(Some(invalid_request()));
    }
    let responses: Vec<_> = batch.iter().filter_map(|v| one(v, limits, &mut handler)).collect();
    Ok((!responses.is_empty()).then_some(Value::Array(responses)))
}
/// Process a permissive legacy JSON-RPC envelope. This mode accepts arbitrary IDs and
/// parameter shapes, ignores response envelopes, and retains legacy diagnostic messages.
/// The caller supplies dispatch and notification callbacks, and should bound transport input.
pub fn process_legacy<F, N>(value: &Value, on_request: F, on_notification: N) -> Option<Value>
where
    F: FnMut(Request<'_>) -> Result<Value, RpcError>,
    N: FnMut(Request<'_>),
{
    process_legacy_with(value, Callbacks { on_request, on_notification })
}

/// Process a permissive legacy JSON-RPC envelope as [`process_legacy`] does, dispatching
/// requests and notifications to one [`Handler`].
pub fn process_legacy_with<H: Handler>(value: &Value, mut handler: H) -> Option<Value> {
    fn one<H: Handler>(value: &Value, handler: &mut H) -> Option<Value> {
        let Some(object) = value.as_object() else { return Some(error_response(Value::Null, RpcError::new(-32600, "message must be an object"))) };
        let id = object.get("id");
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            if object.contains_key("result") || object.contains_key("error") {
                return None;
            }
            return Some(error_response(id.cloned().unwrap_or(Value::Null), RpcError::new(-32600, "missing `method`")));
        };
        let request = Request { id, method, params: object.get("params") };
        let Some(id) = id else {
            handler.notification(request);
            return None;
        };
        Some(match handler.request(request) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(error) => error_response(id.clone(), error),
        })
    }
    if let Some(batch) = value.as_array() {
        if batch.is_empty() {
            return Some(error_response(Value::Null, RpcError::new(-32600, "empty batch")));
        }
        let replies: Vec<_> = batch.iter().filter_map(|v| one(v, &mut handler)).collect();
        (!replies.is_empty()).then_some(Value::Array(replies))
    } else {
        one(value, &mut handler)
    }
}

#[cfg(test)]
mod legacy_tests {
    use super::*;
    #[test]
    fn preserves_legacy_batch_routing_and_errors() {
        let input = json!([{"id": [1], "method":"echo", "params":5}, {"method":"notice"}, {"result":1}, {}, 7]);
        let seen = std::cell::Cell::new(false);
        let result = process_legacy(&input, |r| Ok(r.params.cloned().unwrap_or(Value::Null)), |_| seen.set(true)).unwrap();
        assert!(seen.get());
        assert_eq!(
            result,
            json!([{"jsonrpc":"2.0","id":[1],"result":5}, {"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"missing `method`"}}, {"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"message must be an object"}}])
        );
        assert_eq!(process_legacy(&json!([]), |_| Ok(Value::Null), |_| {}).unwrap()["error"]["message"], "empty batch");
    }
}
