#![cfg(target_arch = "wasm32")]
//! Node tests for the page part: a canned control channel that records what it is asked, and the
//! `window.<app>` functions called the way a page calls them. No browser.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::{Closure, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test;

use craft_web_api::methods::Methods;
use craft_web_api::page::{PageApi, Request};

thread_local! {
    /// (method, params) pairs recorded by the fake control channel.
    static REQUESTS: RefCell<Vec<(String, Value)>> = const { RefCell::new(Vec::new()) };
}

fn reset() {
    REQUESTS.with(|r| r.borrow_mut().clear());
    let _ = js_sys::Reflect::delete_property(&js_sys::global(), &"vectorcraft".into());
}

/// A control channel that always answers `reply`.
fn canned_request(reply: Value) -> Request {
    Rc::new(move |_method: String, _params: Value| {
        let reply = reply.clone();
        Box::pin(async move { reply })
    })
}

/// A control channel that records every call and always answers `reply`.
fn recording_request(reply: Value) -> Request {
    Rc::new(move |method: String, params: Value| {
        REQUESTS.with(|r| r.borrow_mut().push((method.clone(), params)));
        let reply = reply.clone();
        Box::pin(async move { reply })
    })
}

fn to_value(v: &JsValue) -> Value {
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn api() -> js_sys::Object {
    js_sys::Reflect::get(&js_sys::global(), &"vectorcraft".into()).expect("window.vectorcraft").unchecked_into()
}

fn has(api: &js_sys::Object, name: &str) -> bool {
    js_sys::Reflect::has(api, &JsValue::from_str(name)).unwrap_or(false)
}

/// Call `window.vectorcraft.<name>(args…)` and await its promise.
async fn call(name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let api = api();
    let f: js_sys::Function = js_sys::Reflect::get(&api, &JsValue::from_str(name)).expect(name).unchecked_into();
    let promise: js_sys::Promise = f.apply(&api, &args.iter().collect::<js_sys::Array>()).expect("call").unchecked_into();
    JsFuture::from(promise).await
}

fn obj(json: &str) -> JsValue {
    js_sys::JSON::parse(json).expect("valid JSON")
}

#[wasm_bindgen_test]
async fn install_keeps_existing_functions() {
    reset();
    let g = js_sys::global();
    let existing = js_sys::Object::new();
    let marker = Closure::<dyn Fn()>::new(|| {}).into_js_value();
    js_sys::Reflect::set(&existing, &"execute".into(), &marker).expect("set execute");
    js_sys::Reflect::set(&g, &"vectorcraft".into(), &existing.into()).expect("set window.vectorcraft");

    let page = Rc::new(PageApi::new("vectorcraft", Methods::default(), canned_request(json!({"ok": true, "result": {}}))));
    page.install().expect("install");

    let api = api();
    let kept = js_sys::Reflect::get(&api, &"execute".into()).expect("execute");
    assert_eq!(kept, marker, "an app's own execute is kept");
    for name in ["request", "execute", "commands", "docInspect", "uiInspect", "screenshot"] {
        assert!(has(&api, name), "{name} must be added");
    }
}

#[wasm_bindgen_test]
async fn methods_table_decides_which_functions_exist() {
    reset();
    let only = Methods { commands: Some("c.commands"), ..Methods::NONE };
    Rc::new(PageApi::new("vectorcraft", only, canned_request(json!({"ok": true, "result": {}})))).install().expect("install");
    let api = api();
    assert!(has(&api, "request"), "request is always installed");
    assert!(has(&api, "commands"));
    for name in ["execute", "docInspect", "uiInspect", "screenshot"] {
        assert!(!has(&api, name), "{name} has no method and must not be installed");
    }
}

#[wasm_bindgen_test]
async fn functions_send_their_methods() {
    reset();
    let page = Rc::new(PageApi::new("vectorcraft", Methods::default(), recording_request(json!({"ok": true, "result": {"done": 1}}))));
    page.install().expect("install");

    let out = call("execute", &["object.rectangle".into(), obj(r#"{"width": 100}"#)]).await.expect("execute resolves");
    assert_eq!(to_value(&out), json!({"done": 1}), "a success resolves with the result");
    call("commands", &[]).await.expect("commands");
    call("docInspect", &[obj(r#"{"depth": 2}"#)]).await.expect("docInspect");
    call("uiInspect", &[]).await.expect("uiInspect");
    call("screenshot", &[]).await.expect("screenshot");
    call("screenshot", &[obj(r#"{"data": false, "scale": 2}"#)]).await.expect("screenshot with data");
    call("request", &["custom.method".into(), obj(r#"{"x": 1}"#)]).await.expect("request");

    let calls = REQUESTS.with(|r| r.borrow().clone());
    assert_eq!(
        calls,
        [
            ("engine.execute".to_string(), json!({"command": "object.rectangle", "params": {"width": 100}})),
            ("engine.commands".to_string(), json!({})),
            ("document.inspect".to_string(), json!({"depth": 2})),
            ("ui.inspect".to_string(), json!({})),
            ("ui.screenshot".to_string(), json!({"data": true})),
            ("ui.screenshot".to_string(), json!({"data": false, "scale": 2})),
            ("custom.method".to_string(), json!({"x": 1})),
        ]
    );
}

#[wasm_bindgen_test]
async fn error_replies_reject_with_the_error_text() {
    reset();
    Rc::new(PageApi::new("vectorcraft", Methods::default(), canned_request(json!({"ok": false, "error": "boom"})))).install().expect("install");
    let err = match call("commands", &[]).await {
        Err(err) => err,
        Ok(_) => panic!("an error reply must reject"),
    };
    assert_eq!(String::from(err.unchecked_into::<js_sys::Error>().message()), "boom");
}

#[wasm_bindgen_test]
async fn timeout_gives_an_error_saying_so() {
    reset();
    // A control channel that never answers: it waits on a promise that never settles. (Looping on
    // resolved promises instead would starve the timer queue, so the page's timeout could never fire.)
    let pending: Request = Rc::new(|_method: String, _params: Value| {
        Box::pin(async {
            let _ = JsFuture::from(js_sys::Promise::new(&mut |_, _| {})).await;
            Value::Null
        })
    });
    let page = Rc::new(PageApi::new("vectorcraft", Methods::default(), pending).with_timeout(10));
    page.install().expect("install");
    let err = match call("execute", &["object.rectangle".into(), js_sys::Object::new().into()]).await {
        Err(err) => err,
        Ok(_) => panic!("a never-answering app must time out"),
    };
    let message = String::from(err.unchecked_into::<js_sys::Error>().message());
    assert!(message.contains("vectorcraft did not answer engine.execute within 0.01 s"), "{message}");
}
