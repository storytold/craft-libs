//! Typed wrappers over `craft_host.call` (ABI.md section 4).
//!
//! [`call_json`] is the one function that talks to the host. It applies the retry rule: on
//! `E_TOO_SMALL` it calls again with a buffer of the size the host asked for. Every other helper in
//! this module is a request shape plus a reply shape.

use craft_plugin_abi::Op;
use craft_plugin_abi::ops::unpack_call_return;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::backend;
use crate::error::HostError;

/// First reply buffer. Most replies are far smaller.
const FIRST_BUFFER: usize = 4096;
/// The SDK refuses a reply larger than this, so a broken host cannot make the guest allocate without bound.
const MAX_REPLY: usize = 16 * 1024 * 1024;

/// Call `op` with a JSON request and parse the JSON reply. An empty reply is `Value::Null`.
pub fn call_json(op: Op, request: &Value) -> Result<Value, HostError> {
    // An op with no request takes empty input (ABI.md section 4), not the text "null".
    let input = if request.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(request).map_err(|e| HostError::BadReply(format!("request does not serialize: {e}")))?
    };
    let mut capacity = FIRST_BUFFER;
    // E_TOO_SMALL returns the size needed (ABI.md section 4). The guest calls again with that size. Once is enough.
    for attempt in 0..2 {
        let mut buf = vec![0u8; capacity];
        let packed = backend::call(op.number(), &input, &mut buf).ok_or(HostError::NoHost)?;
        let (status, count) = unpack_call_return(packed);
        if status == 0 {
            let body =
                buf.get(..count as usize).ok_or_else(|| HostError::BadReply(format!("host claims {count} bytes in a {capacity} byte buffer")))?;
            if body.is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_slice(body).map_err(|e| HostError::BadReply(e.to_string()));
        }
        let err = HostError::from_status(status, count);
        match err {
            HostError::TooSmall { required } if attempt == 0 && required > capacity && required <= MAX_REPLY => capacity = required,
            other => return Err(other),
        }
    }
    Err(HostError::BadReply("retry loop ended without an answer".into()))
}

fn call_as<T: DeserializeOwned>(op: Op, request: &Value) -> Result<T, HostError> {
    serde_json::from_value(call_json(op, request)?).map_err(|e| HostError::BadReply(format!("{}: {e}", op.name())))
}

fn to_value<T: Serialize + ?Sized>(v: &T) -> Result<Value, HostError> {
    serde_json::to_value(v).map_err(|e| HostError::BadReply(format!("value does not serialize: {e}")))
}

/// Why an op that can fail on the far side gave no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    /// The call itself failed. For `cmd.execute`, `E_BAD_ARG` means an unknown id and `E_DENIED` an id outside the grant.
    Host(HostError),
    /// The call reached the far side and it answered `{"error": "..."}`.
    Failed(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::Host(e) => write!(f, "{e}"),
            OpError::Failed(m) => write!(f, "failed: {m}"),
        }
    }
}

impl std::error::Error for OpError {}

impl From<HostError> for OpError {
    fn from(e: HostError) -> Self {
        OpError::Host(e)
    }
}

/// Split `{"error": ...}` from the rest of a reply.
fn check_error(reply: Value) -> Result<Value, OpError> {
    match reply.get("error") {
        Some(message) => Err(OpError::Failed(message.as_str().map_or_else(|| message.to_string(), str::to_owned))),
        None => Ok(reply),
    }
}

/// The `result` field of a reply, or the error it carries.
fn take_result(reply: Value, op: &str) -> Result<Value, OpError> {
    take_field(reply, "result", op)
}

fn take_field(reply: Value, field: &str, op: &str) -> Result<Value, OpError> {
    match check_error(reply)? {
        Value::Object(mut map) => map.remove(field).ok_or_else(|| OpError::Host(HostError::BadReply(format!("{op} reply has no {field}")))),
        other => Err(OpError::Host(HostError::BadReply(format!("{op} reply is not an object: {other}")))),
    }
}

/// `host.log` severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// `host.log`. The host rate limits it (burst 32, 64 per second) and cuts a message at 4096 bytes.
pub fn log(level: Level, msg: &str) -> Result<(), HostError> {
    call_json(Op::HostLog, &json!({ "level": level.name(), "msg": msg })).map(|_| ())
}

/// `host.log` at `info`. A failed log call is not worth failing a handler, so this returns nothing.
/// Use [`log`] to see the error.
pub fn info(msg: &str) {
    let _ = log(Level::Info, msg);
}

/// `host.log` at `warn`.
pub fn warn(msg: &str) {
    let _ = log(Level::Warn, msg);
}

/// `host.log` at `error`.
pub fn error(msg: &str) {
    let _ = log(Level::Error, msg);
}

/// `host.log` at `debug`.
pub fn debug(msg: &str) {
    let _ = log(Level::Debug, msg);
}

/// The plugin part of `host.info`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct PluginIdentity {
    pub id: String,
    pub generation: u64,
}

/// `host.info`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct HostInfo {
    pub abi: u32,
    pub app: String,
    pub version: String,
    pub engine: String,
    pub plugin: PluginIdentity,
    pub ops: Vec<String>,
}

/// `host.info`: the app, the engine, this plugin's id and generation, and the ops it declared.
pub fn host_info() -> Result<HostInfo, HostError> {
    call_as(Op::HostInfo, &Value::Null)
}

/// `host.cancelled`: true once the user cancelled the running call.
pub fn cancelled() -> Result<bool, HostError> {
    #[derive(serde::Deserialize)]
    struct Reply {
        cancelled: bool,
    }
    call_as::<Reply>(Op::HostCancelled, &Value::Null).map(|r| r.cancelled)
}

/// `host.progress`. Returns `Err(HostError::Cancelled)` once the user cancelled, so a long loop can
/// write `host::progress(f, None)?` and stop.
pub fn progress(fraction: f64, message: Option<&str>) -> Result<(), HostError> {
    let mut req = json!({ "fraction": fraction });
    if let Some(m) = message {
        req["message"] = json!(m);
    }
    call_json(Op::HostProgress, &req).map(|_| ())
}

/// Commands the app runs for this plugin: `cmd.list`, `cmd.execute`.
pub mod cmd {
    use super::{DeserializeOwned, HostError, Op, OpError, Serialize, Value, call_as, call_json, json, take_result, to_value};

    /// One row of `cmd.list`.
    #[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
    pub struct CommandInfo {
        pub id: String,
        pub label: String,
    }

    /// Why `cmd.execute` gave no result.
    pub type CommandError = OpError;

    /// `cmd.list`: only the commands the plugin's grants allow.
    pub fn list() -> Result<Vec<CommandInfo>, HostError> {
        #[derive(serde::Deserialize)]
        struct Reply {
            commands: Vec<CommandInfo>,
        }
        call_as::<Reply>(Op::CmdList, &Value::Null).map(|r| r.commands)
    }

    /// `cmd.execute`: run one app command as one undo step. Returns its `result`.
    pub fn execute<P: Serialize + ?Sized>(id: &str, params: &P) -> Result<Value, CommandError> {
        let reply = call_json(Op::CmdExecute, &json!({ "id": id, "params": to_value(params)? }))?;
        take_result(reply, "cmd.execute")
    }

    /// [`execute`], then parse `result` as `R`.
    pub fn execute_as<R: DeserializeOwned, P: Serialize + ?Sized>(id: &str, params: &P) -> Result<R, CommandError> {
        let value = execute(id, params)?;
        serde_json::from_value(value).map_err(|e| OpError::Host(HostError::BadReply(format!("{id}: {e}"))))
    }
}

/// The plugin's own key-value store: `state.get`, `state.put`. Quota 1 MiB.
pub mod state {
    use super::{DeserializeOwned, HostError, Op, Serialize, Value, call_as, call_json, json, to_value};

    /// `state.get`: `None` when the key is not set.
    pub fn get(key: &str) -> Result<Option<Value>, HostError> {
        #[derive(serde::Deserialize)]
        struct Reply {
            found: bool,
            #[serde(default)]
            value: Option<Value>,
        }
        let reply: Reply = call_as(Op::StateGet, &json!({ "key": key }))?;
        Ok(if reply.found { Some(reply.value.unwrap_or(Value::Null)) } else { None })
    }

    /// [`get`], then parse the value as `T`.
    pub fn get_as<T: DeserializeOwned>(key: &str) -> Result<Option<T>, HostError> {
        match get(key)? {
            Some(v) => serde_json::from_value(v).map(Some).map_err(|e| HostError::BadReply(format!("state key {key:?}: {e}"))),
            None => Ok(None),
        }
    }

    /// `state.put`. `E_LIMIT` once the quota is full.
    pub fn put<T: Serialize + ?Sized>(key: &str, value: &T) -> Result<(), HostError> {
        call_json(Op::StatePut, &json!({ "key": key, "value": to_value(value)? })).map(|_| ())
    }
}

/// `doc.query`: a snapshot of the open document. The reply shape belongs to the app.
pub mod doc {
    use super::{HostError, Op, Value, call_json};

    /// Send `query` (for example `{"scope": "metadata"}`). The host checks the `doc.read` scope.
    pub fn query(query: &Value) -> Result<Value, HostError> {
        call_json(Op::DocQuery, query)
    }
}

/// `time.now`. Returns 0 without the `clock` permission.
pub mod clock {
    use super::{HostError, Op, Value, call_as};

    pub fn now_ms() -> Result<u64, HostError> {
        #[derive(serde::Deserialize)]
        struct Reply {
            unix_ms: u64,
        }
        call_as::<Reply>(Op::TimeNow, &Value::Null).map(|r| r.unix_ms)
    }
}

/// `random.bytes`. A fixed host-seeded stream without the `random` permission.
pub mod random {
    use super::{HostError, Op, call_as, json};

    /// 0 to 4096 bytes.
    pub fn bytes(len: usize) -> Result<Vec<u8>, HostError> {
        #[derive(serde::Deserialize)]
        struct Reply {
            hex: String,
        }
        let reply: Reply = call_as(Op::RandomBytes, &json!({ "len": len }))?;
        decode_hex(&reply.hex)
    }

    fn decode_hex(text: &str) -> Result<Vec<u8>, HostError> {
        let bad = || HostError::BadReply(format!("random.bytes: {text:?} is not hex"));
        if !text.len().is_multiple_of(2) {
            return Err(bad());
        }
        text.as_bytes()
            .chunks(2)
            .map(|pair| {
                let digits = std::str::from_utf8(pair).map_err(|_| bad())?;
                u8::from_str_radix(digits, 16).map_err(|_| bad())
            })
            .collect()
    }
}

/// Craft Link: `link.call`, `link.apps` (needs `link.call:<app>` for each app).
pub mod link {
    use super::{HostError, Op, OpError, Serialize, Value, call_json, json, take_field, take_result, to_value};

    /// A running app this plugin may call.
    #[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
    pub struct LinkApp {
        pub app: String,
        pub app_version: String,
    }

    /// Call `method` on the running `app`. Returns the peer's `result`.
    ///
    /// `link.call:<app>` forwards only `link.resolve`, `asset.describe`, `link.refresh` and
    /// `link.focus`. `engine.execute` also needs `link.exec:<app>:<command-glob>` for the command in
    /// `params.command`, and never reaches a host command (`plugin.app.*`).
    /// A missing grant, a refused method or a dead peer comes back as `OpError::Failed` with the reason in the text.
    pub fn call<P: Serialize + ?Sized>(app: &str, method: &str, params: &P) -> Result<Value, OpError> {
        let reply = call_json(Op::LinkCall, &json!({ "app": app, "method": method, "params": to_value(params)? }))?;
        take_result(reply, "link.call")
    }

    /// Running apps this plugin holds `link.call:<app>` for.
    pub fn apps() -> Result<Vec<LinkApp>, OpError> {
        let reply = call_json(Op::LinkApps, &Value::Null)?;
        let apps = take_field(reply, "apps", "link.apps")?;
        serde_json::from_value(apps).map_err(|e| OpError::Host(HostError::BadReply(format!("link.apps: {e}"))))
    }
}
