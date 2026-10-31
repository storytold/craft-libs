//! The browser's [`Fetcher`]: `fetch` with Subresource Integrity, and `setTimeout`.

use std::sync::Arc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::loader::{self, Done, Fetcher, Wake};
use crate::fonts::Stack;

/// Install `stack`, add the files fetched so far, and fetch the UI language's faces.
pub(in crate::fonts) fn install(ctx: &egui::Context, stack: Stack) {
    loader::install(ctx, stack, Arc::new(Browser));
}

pub(in crate::fonts) fn loading(ctx: &egui::Context) -> bool {
    loader::loading(ctx)
}

struct Browser;

impl Fetcher for Browser {
    fn fetch(&self, url: &'static str, integrity: &'static str, done: Done) {
        wasm_bindgen_futures::spawn_local(async move {
            let result = fetch_file(url, integrity).await;
            if let Err(error) = &result {
                web_sys::console::warn_1(&format!("craft-ui: cannot fetch the font {url}: {error}").into());
            }
            done(result);
        });
    }

    fn now(&self) -> f64 {
        js_sys::Date::now() / 1000.0
    }

    fn after(&self, seconds: f64, wake: Wake) {
        let Some(window) = web_sys::window() else { return };
        let callback = Closure::once_into_js(wake);
        let millis = (seconds * 1000.0) as i32;
        // Without the timer, the face is retried when the next fetch finishes.
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), millis);
    }
}

async fn fetch_file(url: &str, integrity: &str) -> Result<Vec<u8>, String> {
    // An Error's message only: its Debug form carries the whole JS stack. The browser logs the
    // cause (network, integrity) itself.
    let js = |e: JsValue| match e.dyn_ref::<js_sys::Error>() {
        Some(error) => String::from(error.message()),
        None => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    };
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    if !integrity.is_empty() {
        init.set_integrity(integrity);
    }
    let response: web_sys::Response = JsFuture::from(window.fetch_with_str_and_init(url, &init)).await.map_err(js)?.dyn_into().map_err(js)?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(js)?).await.map_err(js)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}
