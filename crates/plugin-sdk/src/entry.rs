//! What a handler receives and returns, and the adapters `#[craft_plugin]` wires between those
//! types and the JSON the host sends and reads.

use std::fmt::Display;

use craft_plugin_abi::ProcessDescriptor;
use craft_plugin_abi::descriptor::Kind;
use craft_plugin_abi::format::{self as abi_format, ImageDoc};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::ui::Widget;

/// The adapter for a `craft_invoke` contribution: request JSON and blob in, response JSON out.
/// `Ok(None)` is an empty response. `Err` means the request was malformed (`E_BAD_ARG`).
pub type ControlFn = fn(&Value, &[u8]) -> Result<Option<Value>, String>;

/// The adapter for a `craft_process` contribution.
pub type DataFn = fn(&mut Band<'_>) -> Result<(), String>;

fn object<'a>(req: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    req.as_object().ok_or_else(|| format!("{what} request is not a JSON object"))
}

// ---- command and menu ---------------------------------------------------------------------

/// `{"params": {...}, "origin": "ui"|"script"|"unknown"|...}`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandCall {
    pub params: Value,
    pub origin: String,
}

impl CommandCall {
    /// Parse `params` as `T`. The error text is fit to return from a handler.
    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_value(self.params.clone()).map_err(|e| format!("bad params: {e}"))
    }
}

/// What a command handler may return. `Ok(v)` becomes `{"result": v}`, `Err(e)` becomes
/// `{"error": "e"}`. The host undoes the call on `{"error"}`.
pub trait CommandReturn {
    fn into_response(self) -> Value;
}

impl<T: Serialize, E: Display> CommandReturn for Result<T, E> {
    fn into_response(self) -> Value {
        match self {
            Ok(v) => match serde_json::to_value(v) {
                Ok(v) => json!({ "result": v }),
                Err(e) => json!({ "error": format!("result does not serialize: {e}") }),
            },
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}

#[doc(hidden)]
pub fn command<R: CommandReturn>(req: &Value, handler: impl FnOnce(CommandCall) -> R) -> Result<Option<Value>, String> {
    let obj = object(req, "command")?;
    let call = CommandCall {
        params: obj.get("params").cloned().unwrap_or(Value::Null),
        origin: obj.get("origin").and_then(Value::as_str).unwrap_or("unknown").to_owned(),
    };
    Ok(Some(handler(call).into_response()))
}

// ---- hook ---------------------------------------------------------------------------------

/// The `call` object of a hook request.
#[derive(Clone, Debug, PartialEq, Default, serde::Deserialize)]
#[serde(default)]
pub struct CallInfo {
    pub id: String,
    pub params: Value,
    pub depth: u32,
    pub origin: String,
    pub seq: u64,
}

/// A hook request. Which fields are set depends on the hook's phase: `before` gets `call`,
/// `after` gets `call` and `outcome`, `change` gets `change`.
#[derive(Clone, Debug, PartialEq)]
pub struct HookCall {
    pub call: Option<CallInfo>,
    pub outcome: Option<String>,
    pub change: Option<Value>,
}

/// A hook's answer. Only `before` hooks read it. The host ignores it for `after` and `change`.
#[derive(Clone, Debug, PartialEq)]
pub enum HookVerdict {
    /// `{}`: let the command run.
    Pass,
    /// `{"rewrite": ...}`: run the command with these arguments instead.
    Rewrite(Value),
    /// `{"veto": "reason"}`: stop the command.
    Veto(String),
}

#[doc(hidden)]
pub fn hook(req: &Value, handler: impl FnOnce(HookCall) -> HookVerdict) -> Result<Option<Value>, String> {
    let obj = object(req, "hook")?;
    let call = match obj.get("call") {
        Some(c) => Some(serde_json::from_value(c.clone()).map_err(|e| format!("hook call: {e}"))?),
        None => None,
    };
    let hook_call = HookCall { call, outcome: obj.get("outcome").and_then(Value::as_str).map(str::to_owned), change: obj.get("change").cloned() };
    Ok(Some(match handler(hook_call) {
        HookVerdict::Pass => json!({}),
        HookVerdict::Rewrite(v) => json!({ "rewrite": v }),
        HookVerdict::Veto(reason) => json!({ "veto": reason }),
    }))
}

// ---- panel --------------------------------------------------------------------------------

/// `{"event": "open"|"close"|"action", "widget": "id", "value": any}`.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelEvent {
    pub event: String,
    pub widget: Option<String>,
    pub value: Value,
}

/// What a panel handler may return: a tree, `Some(tree)`, or `None` for "no change" (`{}`).
pub trait PanelReturn {
    fn into_tree(self) -> Option<Widget>;
}

impl PanelReturn for Widget {
    fn into_tree(self) -> Option<Widget> {
        Some(self)
    }
}

impl PanelReturn for Option<Widget> {
    fn into_tree(self) -> Option<Widget> {
        self
    }
}

#[doc(hidden)]
pub fn panel<R: PanelReturn>(req: &Value, handler: impl FnOnce(PanelEvent) -> R) -> Result<Option<Value>, String> {
    let obj = object(req, "panel")?;
    let event = PanelEvent {
        event: obj.get("event").and_then(Value::as_str).ok_or("panel request has no event")?.to_owned(),
        widget: obj.get("widget").and_then(Value::as_str).map(str::to_owned),
        value: obj.get("value").cloned().unwrap_or(Value::Null),
    };
    Ok(Some(match handler(event).into_tree() {
        Some(tree) => json!({ "tree": tree.build().map_err(|e| e.to_string())? }),
        None => json!({}),
    }))
}

// ---- format -------------------------------------------------------------------------------

/// Which way a `format` request goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatOp {
    /// `data` is the file. Answer with [`FormatOutput::Image`].
    Read,
    /// `data` is `width * height * 4` bytes of RGBA8. Answer with [`FormatOutput::Bytes`].
    Write,
}

/// A `format` request (ABI.md section 6, see [`craft_plugin_abi::format`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatCall<'a> {
    pub op: FormatOp,
    /// The file name the user chose, with its extension.
    pub name: String,
    /// The lower case extension that selected this plugin.
    pub ext: String,
    /// The size of the image to write. Zero for a read.
    pub width: u32,
    pub height: u32,
    /// The file bytes (read) or the RGBA8 pixels (write).
    pub data: &'a [u8],
}

impl FormatCall<'_> {
    /// For a write: the pixels as a checked [`ImageDoc`].
    pub fn image(&self) -> Result<ImageDoc, String> {
        ImageDoc::new(self.width, self.height, self.data.to_vec()).map_err(|e| e.to_string())
    }
}

/// What a `format` handler answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatOutput {
    /// The decoded file. Answer to a read.
    Image(ImageDoc),
    /// The encoded file. Answer to a write.
    Bytes(Vec<u8>),
}

impl From<ImageDoc> for FormatOutput {
    fn from(doc: ImageDoc) -> Self {
        FormatOutput::Image(doc)
    }
}

impl From<Vec<u8>> for FormatOutput {
    fn from(bytes: Vec<u8>) -> Self {
        FormatOutput::Bytes(bytes)
    }
}

/// What a format handler may return: `Result<O, E>` with `O` convertible to [`FormatOutput`].
/// `Err` becomes `{"error": "..."}`, which the host reports as the plugin's own error.
pub trait FormatReturn {
    fn into_output(self) -> Result<FormatOutput, String>;
}

impl<O: Into<FormatOutput>, E: Display> FormatReturn for Result<O, E> {
    fn into_output(self) -> Result<FormatOutput, String> {
        self.map(Into::into).map_err(|e| e.to_string())
    }
}

#[doc(hidden)]
pub fn format<'a, R: FormatReturn>(req: &Value, blob: &'a [u8], handler: impl FnOnce(FormatCall<'a>) -> R) -> Result<Option<Value>, String> {
    let obj = object(req, "format")?;
    let op = match obj.get("op").and_then(Value::as_str) {
        Some(abi_format::OP_READ) => FormatOp::Read,
        Some(abi_format::OP_WRITE) => FormatOp::Write,
        other => return Err(format!("format request has op {other:?}, wanted \"read\" or \"write\"")),
    };
    let text = |k: &str| obj.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let dim = |k: &str| obj.get("doc").and_then(|d| d.get(k)).and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let call = FormatCall { op, name: text("name"), ext: text("ext"), width: dim("width"), height: dim("height"), data: blob };
    Ok(Some(match (op, handler(call).into_output()) {
        (_, Err(e)) => json!({ "error": e }),
        (FormatOp::Read, Ok(FormatOutput::Image(doc))) => abi_format::read_response(&doc),
        (FormatOp::Write, Ok(FormatOutput::Bytes(bytes))) => abi_format::write_response(&bytes),
        (FormatOp::Read, Ok(FormatOutput::Bytes(_))) => json!({ "error": "a read must answer with an image" }),
        (FormatOp::Write, Ok(FormatOutput::Image(_))) => json!({ "error": "a write must answer with bytes" }),
    }))
}

// ---- image bands (effect, filter, object-filter, live-effect) -----------------------------

/// One band of an image, as the host hands it over (ABI.md section 7).
pub struct Band<'a> {
    pub desc: &'a ProcessDescriptor,
    /// The `params` JSON the host validated against the contribution's schema.
    pub params: Value,
    pub input: &'a [u8],
    pub output: &'a mut [u8],
}

impl Band<'_> {
    pub fn width(&self) -> usize {
        self.desc.w as usize
    }

    pub fn height(&self) -> usize {
        self.desc.h as usize
    }

    pub fn stride(&self) -> usize {
        self.desc.stride_bytes as usize
    }

    pub fn channels(&self) -> usize {
        self.desc.channels as usize
    }

    /// A number param by name, or `default` when it is absent or not a number.
    pub fn param_f64(&self, name: &str, default: f64) -> f64 {
        self.params.get(name).and_then(Value::as_f64).unwrap_or(default)
    }

    /// Input and output rows, `stride` bytes each. Empty for audio or a zero stride.
    pub fn rows(&mut self) -> impl Iterator<Item = (&[u8], &mut [u8])> {
        let stride = if self.desc.kind == Kind::Image { self.stride().max(1) } else { usize::MAX };
        self.input.chunks(stride).zip(self.output.chunks_mut(stride))
    }
}

/// What an image handler may return: `()` or `Result<(), E>`. An error makes `craft_process`
/// return non-zero, and the host discards the band.
pub trait ImageReturn {
    fn into_result(self) -> Result<(), String>;
}

impl ImageReturn for () {
    fn into_result(self) -> Result<(), String> {
        Ok(())
    }
}

impl<E: Display> ImageReturn for Result<(), E> {
    fn into_result(self) -> Result<(), String> {
        self.map_err(|e| e.to_string())
    }
}

#[doc(hidden)]
pub fn image<R: ImageReturn>(band: &mut Band<'_>, handler: impl FnOnce(&mut Band<'_>) -> R) -> Result<(), String> {
    handler(band).into_result()
}
