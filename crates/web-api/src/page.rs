//! `window.<app>`: the page's command API. wasm32 only. The app hands this crate its control channel
//! as a [`Request`]; the crate never knows the app's internals.
//!
//! Every function goes through one path, [`PageApi::call`], with the page's timeout; replies
//! resolve with the method's `result` or reject with an `Error` carrying `error`.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::{Closure, JsValue};
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::methods::Methods;

/// One control request: method and params in, the protocol's reply object out (`{ok, result}` or
/// `{ok: false, error}`). The app builds this from its control channel — send a `ControlRequest`,
/// await its reply.
pub type Request = Rc<dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Value>>>>;

/// The page surface of one web build: the `window.<app>` object.
pub struct PageApi {
    app: String,
    /// The control method behind each function: the desktop control-server method names this app uses.
    methods: Methods,
    request: Request,
    /// Each control request gets a timeout: if the app does not answer within this many
    /// milliseconds, the error says so (a hidden tab answers when it is shown again).
    timeout_ms: f64,
}

impl PageApi {
    /// `app` is the global name to install (`"vectorcraft"` → `window.vectorcraft`); `methods` is
    /// the control method behind each function ([`Methods::default`] is the desktop control-server
    /// method names).
    pub fn new(app: &str, methods: Methods, request: Request) -> Self {
        Self { app: app.to_string(), methods, request, timeout_ms: 30_000.0 }
    }

    /// Set the timeout for every control request, in milliseconds (default 30 s).
    pub fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms as f64;
        self
    }

    /// Install `window.<app>` = `{request, execute, commands, docInspect, uiInspect, screenshot}` —
    /// each a Promise-returning function that resolves with the method's `result` or rejects with
    /// an `Error` carrying `error`. Only the functions whose control method the app's [`Methods`]
    /// table provides are installed; `request` is always there.
    ///
    /// An existing `window.<app>` is never replaced: the missing functions are added to it, and an
    /// app that already has its own keeps them.
    pub fn install(self: Rc<Self>) -> Result<(), JsValue> {
        let g = global();
        let name = JsValue::from_str(&self.app);
        let existing = get_prop(&g, &self.app);
        let existed = existing.as_ref().is_some_and(|v| v.is_object());
        let target: js_sys::Object = match existing {
            Some(v) if v.is_object() => v.unchecked_into(),
            _ => js_sys::Object::new(),
        };
        let add = |name: &str, f: JsValue| -> Result<(), JsValue> {
            if js_sys::Reflect::has(&target, &JsValue::from_str(name)).unwrap_or(true) {
                return Ok(()); // an app that already has its own function keeps it
            }
            js_sys::Reflect::set(&target, &JsValue::from_str(name), &f).map(|_| ())
        };

        let page = self.clone();
        add(
            "request",
            Closure::<dyn FnMut(String, JsValue) -> js_sys::Promise>::new(move |method: String, params: JsValue| {
                let page = page.clone();
                future_to_promise(async move { request_promise_outcome(&page.call(&method, from_js(&params)).await) })
            })
            .into_js_value(),
        )?;

        if let Some(method) = self.methods.execute {
            let page = self.clone();
            add(
                "execute",
                Closure::<dyn FnMut(String, JsValue) -> js_sys::Promise>::new(move |command: String, params: JsValue| {
                    let page = page.clone();
                    future_to_promise(async move {
                        let params = json!({"command": command, "params": from_js(&params)});
                        request_promise_outcome(&page.call(method, params).await)
                    })
                })
                .into_js_value(),
            )?;
        }

        if let Some(method) = self.methods.commands {
            let page = self.clone();
            add(
                "commands",
                Closure::<dyn FnMut() -> js_sys::Promise>::new(move || {
                    let page = page.clone();
                    future_to_promise(async move { request_promise_outcome(&page.call(method, json!({})).await) })
                })
                .into_js_value(),
            )?;
        }

        for (name, method) in [("docInspect", self.methods.doc_inspect), ("uiInspect", self.methods.ui_inspect)] {
            if let Some(method) = method {
                let page = self.clone();
                add(
                    name,
                    Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(move |params: JsValue| {
                        let page = page.clone();
                        future_to_promise(async move { request_promise_outcome(&page.call(method, from_js(&params)).await) })
                    })
                    .into_js_value(),
                )?;
            }
        }

        if let Some(method) = self.methods.screenshot {
            let page = self.clone();
            add(
                "screenshot",
                Closure::<dyn FnMut(JsValue) -> js_sys::Promise>::new(move |params: JsValue| {
                    let page = page.clone();
                    let params = screenshot_params(from_js(&params));
                    future_to_promise(async move { request_promise_outcome(&page.call(method, params).await) })
                })
                .into_js_value(),
            )?;
        }

        if existed { Ok(()) } else { js_sys::Reflect::set(&g, &name, &target).map(|_| ()) }
    }

    /// One control request: the single path through the app's channel that every function takes,
    /// with the page's timeout. If the app does not answer within the timeout, the error says so —
    /// a hidden tab answers when it is shown again.
    async fn call(&self, method: &str, params: Value) -> Value {
        let timeout = Timeout::new(self.timeout_ms);
        let request = self.request.clone();
        let method = method.to_string();
        let sent = method.clone();
        let reply = future_to_promise(async move {
            let reply = request(sent, params).await;
            Ok(to_js(&reply))
        });
        let raced = js_sys::Promise::race(&js_sys::Array::of2(&reply, &timeout.promise));
        let outcome = JsFuture::from(raced).await;
        drop(timeout); // clear the timer and free its callback once the race is over
        match outcome {
            Err(_) => json!({
                "ok": false,
                "error": format!(
                    "{} did not answer {} within {} s (a hidden tab answers when it is shown again)",
                    self.app, method, self.timeout_ms / 1000.0
                ),
            }),
            Ok(value) => from_js(&value),
        }
    }
}

/// `screenshot` params: `data: true` is added so the reply carries the PNG, unless the caller set
/// `data` itself.
fn screenshot_params(mut params: Value) -> Value {
    match params.as_object_mut() {
        Some(o) => {
            o.entry("data").or_insert(json!(true));
            params
        }
        None => json!({"data": true}),
    }
}

/// A promise that rejects `ms` milliseconds from now. Dropping it clears the timer (harmless once
/// fired) and frees the timer's callback, so a call leaves nothing behind. The timer comes from
/// `setTimeout` on the global object, reached through `Reflect` — no web-sys, so the crate keeps
/// working in any JS realm.
struct Timeout {
    promise: js_sys::Promise,
    id: f64,
    _fire: Option<Closure<dyn FnMut()>>,
}

impl Timeout {
    fn new(ms: f64) -> Self {
        let mut fire = None;
        let mut id = f64::INFINITY;
        // `Promise::new` runs its executor before returning, so `fire` and `id` are set here.
        let promise = js_sys::Promise::new(&mut |_resolve: js_sys::Function, reject: js_sys::Function| {
            let callback = Closure::<dyn FnMut()>::new(move || {
                let _ = reject.call1(&JsValue::NULL, &JsValue::NULL);
            });
            id = set_timeout(callback.as_ref().clone(), ms);
            fire = Some(callback);
        });
        Self { promise, id, _fire: fire }
    }
}

impl Drop for Timeout {
    fn drop(&mut self) {
        clear_timeout(self.id);
    }
}

/// The `setTimeout` global, reached through the global object + `Reflect` (no web-sys). Returns the
/// timer id; a missing `setTimeout` yields an id of infinity, which [`clear_timeout`] ignores.
fn set_timeout(callback: JsValue, ms: f64) -> f64 {
    let g = global();
    get_prop(&g, "setTimeout")
        .filter(|v| v.is_function())
        .and_then(|f| {
            let f: js_sys::Function = f.unchecked_into();
            f.call2(&g, &callback, &JsValue::from_f64(ms)).ok()
        })
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::INFINITY)
}

/// Clear a `setTimeout` id, if the environment has `clearTimeout`.
fn clear_timeout(id: f64) {
    let g = global();
    if let Some(f) = get_prop(&g, "clearTimeout").filter(|v| v.is_function()) {
        let f: js_sys::Function = f.unchecked_into();
        let _ = f.call1(&g, &JsValue::from_f64(id));
    }
}

/// The `Promise` outcome of a `window.<app>` call: resolve with `result`, reject with an `Error`
/// carrying `error`.
fn request_promise_outcome(reply: &Value) -> Result<JsValue, JsValue> {
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(to_js(reply.get("result").unwrap_or(&Value::Null)))
    } else {
        Err(js_sys::Error::new(&error_text(reply)).into())
    }
}

/// A reply's `error` as displayable text.
fn error_text(reply: &Value) -> String {
    match reply.get("error") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "error".to_string()),
        None => "error".to_string(),
    }
}

/// The global object (`window` in a browser, `globalThis` in Node): `window.<app>`, `setTimeout`
/// and `clearTimeout` live on it.
fn global() -> js_sys::Object {
    js_sys::global()
}

fn get_prop(target: &js_sys::Object, name: &str) -> Option<JsValue> {
    js_sys::Reflect::get(target, &JsValue::from_str(name)).ok()
}

/// JSON → JS. VectorCraft's `to_js`.
fn to_js(v: &Value) -> JsValue {
    js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL)
}

/// JS → JSON. VectorCraft's `from_js`; absent values become an empty object.
fn from_js(v: &JsValue) -> Value {
    if v.is_undefined() || v.is_null() {
        return json!({});
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}
