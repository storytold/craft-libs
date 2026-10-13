//! The browser side (feature `web`, wasm32): [`Client`] sends the [`crate::protocol`] requests
//! with `fetch` to a path on the page's own origin, which the web server proxies to
//! `craft-updated`. No cookies are involved: the administrator's session is a bearer token held
//! in this page's memory only, so it can't be ridden by another site and ends with the tab.

use crate::client::{Backend, Reply, parse_reply};
use crate::protocol::{self, LoginRequest};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::{JsCast as _, JsValue};

/// Talks to `craft-updated` at `base` (relative to the page, such as `craft-update/`).
pub struct Client {
    base: String,
    replies: Rc<RefCell<Vec<Reply>>>,
    wake: Rc<dyn Fn()>,
}

impl Client {
    /// `wake` is called when an answer arrives (request a repaint there).
    pub fn new(base: &str, wake: impl Fn() + 'static) -> Client {
        let mut base = base.to_string();
        if !base.ends_with('/') {
            base.push('/');
        }
        Client { base, replies: Rc::default(), wake: Rc::new(wake) }
    }

    fn send(&self, method: &'static str, path: &str, body: Option<String>, token: Option<String>, make: Option<fn(u16, &str) -> Reply>) {
        let url = format!("{}{path}", self.base);
        let replies = Rc::clone(&self.replies);
        let wake = Rc::clone(&self.wake);
        wasm_bindgen_futures::spawn_local(async move {
            let (status, text) = fetch(method, &url, body, token).await.unwrap_or_else(|e| {
                log_warn(&format!("craft-update: {method} {url}: {e}"));
                (0, String::new())
            });
            if let Some(make) = make {
                replies.borrow_mut().push(make(status, &text));
                wake();
            }
        });
    }
}

impl Backend for Client {
    fn request_status(&mut self) {
        self.send("GET", protocol::STATUS, None, None, Some(|s, b| Reply::Status(parse_reply(s, b))));
    }

    fn request_login(&mut self, username: &str, password: &str) {
        let body = serde_json::to_string(&LoginRequest { username: username.into(), password: password.into() }).ok();
        self.send("POST", protocol::LOGIN, body, None, Some(|s, b| Reply::Login(parse_reply(s, b))));
    }

    fn request_update(&mut self, token: &str) {
        self.send("POST", protocol::UPDATE, None, Some(token.into()), Some(|s, b| Reply::Update(parse_reply(s, b))));
    }

    fn request_logout(&mut self, token: &str) {
        // The answer doesn't matter: the page forgets the token either way.
        self.send("POST", protocol::LOGOUT, None, Some(token.into()), None);
    }

    fn poll(&mut self) -> Vec<Reply> {
        std::mem::take(&mut *self.replies.borrow_mut())
    }

    fn reload_page(&mut self) {
        if let Some(window) = web_sys::window() {
            let _ = window.location().reload();
        }
    }
}

fn log_warn(message: &str) {
    web_sys::console::warn_1(&JsValue::from_str(message));
}

async fn fetch(method: &str, url: &str, body: Option<String>, token: Option<String>) -> Result<(u16, String), String> {
    let js = |e: JsValue| format!("{e:?}");
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_cache(web_sys::RequestCache::NoStore);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    if let Some(body) = &body {
        init.set_body(&JsValue::from_str(body));
    }
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let headers = request.headers();
    if body.is_some() {
        headers.set("Content-Type", "application/json").map_err(js)?;
    }
    if let Some(token) = token {
        headers.set("Authorization", &format!("Bearer {token}")).map_err(js)?;
    }
    let window = web_sys::window().ok_or("no window")?;
    let response: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request)).await.map_err(js)?.dyn_into().map_err(js)?;
    let text = wasm_bindgen_futures::JsFuture::from(response.text().map_err(js)?).await.map_err(js)?.as_string().unwrap_or_default();
    Ok((response.status(), text))
}
