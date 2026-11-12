//! Building an API request (RFC 8620 §3.3): capabilities used, method calls with call ids (§3.2), and result
//! references between calls (§3.7).
//!
//! Call ids are chosen by the builder (`c0`, `c1`, …) so that they are unique within the request and valid
//! ids. A [`ResultReference`] can only be made to a call already in the same request, which is the order
//! RFC 8620 §3.7 requires ("a previous method call in the current request").

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};

use crate::error::BuildError;
use crate::id::Id;
use crate::session::CAPABILITY_CORE;

/// The most method calls one request may hold, whatever the server allows.
pub const MAX_CALLS: usize = 1024;
/// The longest method or response name accepted.
pub const MAX_METHOD_NAME_LEN: usize = 255;
/// The longest result reference path accepted.
pub const MAX_PATH_LEN: usize = 1024;
/// The longest capability identifier accepted in `using`.
const MAX_CAPABILITY_LEN: usize = 256;

/// Gives every request its own tag, so that a call id from one request is not taken for one of another.
static NEXT_REQUEST_TAG: AtomicU64 = AtomicU64::new(1);

/// The id of a method call within a request (RFC 8620 §3.2), tied to the request that made it.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallId {
    text: String,
    request: u64,
}

impl CallId {
    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Debug for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CallId({})", self.text)
    }
}

/// A reference to the result of an earlier call (RFC 8620 §3.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultReference {
    result_of: CallId,
    name: String,
    path: String,
}

impl ResultReference {
    /// The call whose result is used.
    pub fn result_of(&self) -> &CallId {
        &self.result_of
    }

    /// The response name required.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The JSON Pointer into the response's arguments.
    pub fn path(&self) -> &str {
        &self.path
    }

    fn to_json(&self) -> Value {
        let mut object = Map::new();
        object.insert("resultOf".to_owned(), Value::String(self.result_of.text.clone()));
        object.insert("name".to_owned(), Value::String(self.name.clone()));
        object.insert("path".to_owned(), Value::String(self.path.clone()));
        Value::Object(object)
    }
}

#[derive(Clone, PartialEq)]
struct Call {
    method: String,
    arguments: Map<String, Value>,
    id: CallId,
}

/// An API request under construction.
#[derive(Clone, PartialEq)]
pub struct ApiRequest {
    tag: u64,
    using: Vec<String>,
    calls: Vec<Call>,
    created_ids: Option<BTreeMap<Id, Id>>,
    response_limit: Option<usize>,
}

fn is_printable_name(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && text.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

/// RFC 6901 §3: a JSON Pointer is empty or a sequence of `/`-prefixed tokens in which `~` only appears as
/// `~0` or `~1`. RFC 8620 §3.7 adds `*` as a token, which needs no special syntax.
fn is_json_pointer(path: &str) -> bool {
    if path.len() > MAX_PATH_LEN || !(path.is_empty() || path.starts_with('/')) {
        return false;
    }
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

impl Default for ApiRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiRequest {
    /// An empty request using the core capability.
    pub fn new() -> Self {
        Self {
            tag: NEXT_REQUEST_TAG.fetch_add(1, Ordering::Relaxed),
            using: vec![CAPABILITY_CORE.to_owned()],
            calls: Vec::new(),
            created_ids: None,
            response_limit: None,
        }
    }

    /// Adds a capability to `using` (once).
    pub fn using(&mut self, capability: &str) -> Result<&mut Self, BuildError> {
        if !is_printable_name(capability, MAX_CAPABILITY_LEN) {
            return Err(BuildError::InvalidCapability);
        }
        if !self.using.iter().any(|known| known == capability) {
            self.using.push(capability.to_owned());
        }
        Ok(self)
    }

    /// The capabilities in `using`.
    pub fn capabilities(&self) -> impl Iterator<Item = &str> {
        self.using.iter().map(String::as_str)
    }

    /// Lowers, for this request only, the largest response body accepted; the HTTP request built from it
    /// carries the limit ([`HttpRequest::max_response_bytes`](crate::HttpRequest::max_response_bytes)). It
    /// cannot exceed the limit for API responses.
    pub fn set_response_limit(&mut self, bytes: usize) {
        self.response_limit = Some(bytes);
    }

    /// The response limit set with [`ApiRequest::set_response_limit`], if any.
    pub fn response_limit(&self) -> Option<usize> {
        self.response_limit
    }

    /// Sets the request's `createdIds` (RFC 8620 §3.3); the response will then carry them back.
    pub fn set_created_ids(&mut self, ids: BTreeMap<Id, Id>) {
        self.created_ids = Some(ids);
    }

    /// Adds a method call and returns its id.
    pub fn add_call(&mut self, method: &str, arguments: Map<String, Value>) -> Result<CallId, BuildError> {
        self.add_call_with_references(method, arguments, Vec::new())
    }

    /// Adds a method call whose arguments include result references: each `(name, reference)` becomes the
    /// argument `#name` (RFC 8620 §3.7). An argument may not be given both ways.
    pub fn add_call_with_references(
        &mut self,
        method: &str,
        mut arguments: Map<String, Value>,
        references: Vec<(&str, ResultReference)>,
    ) -> Result<CallId, BuildError> {
        if !is_printable_name(method, MAX_METHOD_NAME_LEN) {
            return Err(BuildError::InvalidMethodName);
        }
        if self.calls.len() >= MAX_CALLS {
            return Err(BuildError::TooManyCalls { limit: u64::try_from(MAX_CALLS).unwrap_or(u64::MAX) });
        }
        for (name, reference) in references {
            if name.is_empty() || name.starts_with('#') {
                return Err(BuildError::InvalidArgumentName);
            }
            if !self.calls.iter().any(|call| call.id == reference.result_of) {
                return Err(BuildError::UnknownCall);
            }
            let key = format!("#{name}");
            if arguments.contains_key(name) || arguments.contains_key(&key) {
                return Err(BuildError::DuplicateArgument);
            }
            arguments.insert(key, reference.to_json());
        }
        let duplicated = arguments.keys().any(|key| key.strip_prefix('#').is_some_and(|plain| !plain.is_empty() && arguments.contains_key(plain)));
        if duplicated {
            return Err(BuildError::DuplicateArgument);
        }
        let id = CallId { text: format!("c{}", self.calls.len()), request: self.tag };
        self.calls.push(Call { method: method.to_owned(), arguments, id: id.clone() });
        Ok(id)
    }

    /// A reference to the result of `call`, an earlier call of this request (RFC 8620 §3.7).
    pub fn reference(&self, call: &CallId, response_name: &str, path: &str) -> Result<ResultReference, BuildError> {
        if !self.calls.iter().any(|known| known.id == *call) {
            return Err(BuildError::UnknownCall);
        }
        if !is_printable_name(response_name, MAX_METHOD_NAME_LEN) {
            return Err(BuildError::InvalidMethodName);
        }
        if !is_json_pointer(path) {
            return Err(BuildError::InvalidPath);
        }
        Ok(ResultReference { result_of: call.clone(), name: response_name.to_owned(), path: path.to_owned() })
    }

    /// The number of method calls.
    pub fn len(&self) -> usize {
        self.calls.len()
    }

    /// Whether the request has no method call.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// The ids of the calls, in order.
    pub fn call_ids(&self) -> impl Iterator<Item = &CallId> {
        self.calls.iter().map(|call| &call.id)
    }

    /// The request as a JSON text (RFC 8620 §3.3).
    pub fn to_json(&self) -> Result<Vec<u8>, BuildError> {
        if self.calls.is_empty() {
            return Err(BuildError::NoCalls);
        }
        let mut object = Map::new();
        object.insert("using".to_owned(), Value::Array(self.using.iter().cloned().map(Value::String).collect()));
        let calls = self
            .calls
            .iter()
            .map(|call| {
                Value::Array(vec![Value::String(call.method.clone()), Value::Object(call.arguments.clone()), Value::String(call.id.text.clone())])
            })
            .collect();
        object.insert("methodCalls".to_owned(), Value::Array(calls));
        if let Some(created) = &self.created_ids {
            let map = created.iter().map(|(key, value)| (key.as_str().to_owned(), Value::String(value.as_str().to_owned()))).collect();
            object.insert("createdIds".to_owned(), Value::Object(map));
        }
        serde_json::to_vec(&Value::Object(object)).map_err(|_| BuildError::Serialisation)
    }
}

impl fmt::Debug for ApiRequest {
    /// Capabilities, methods and call ids; the arguments may hold personal data and are not shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let calls: Vec<String> = self.calls.iter().map(|call| format!("{} {}", call.id, call.method)).collect();
        f.debug_struct("ApiRequest").field("using", &self.using).field("calls", &calls).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
