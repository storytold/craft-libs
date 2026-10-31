#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! One page command API for every Craft web build.
//!
//! Craft web apps (the `apps/<app>-web` builds of VectorCraft, PhotoCraft, FilmCraft, …) accept
//! JSON control requests `{method, params}` from their page and answer `{ok, result}` or
//! `{ok: false, error}` — the same protocol as the desktop control server. Until now each app kept
//! its own copy of the code that exposes this to the page as `window.<app>`, in a different shape.
//! This crate is the one shared shape.
//!
//! - [`page`] (wasm32 only) installs `window.<app>` — `request(method, params)`,
//!   `execute(command, params)`, `commands()`, `docInspect(params)`, `uiInspect(params)` and
//!   `screenshot(params)`; each function other than `request` is installed only when the app's
//!   [`methods::Methods`] table provides its control method.
//! - [`methods`] is the target-independent table naming the control method behind each function.
//!
//! The app hands the crate its request function (its control channel) and a [`methods::Methods`]
//! table; the crate never knows the app's internals.
//!
//! # How an app wires it
//!
//! ```ignore
//! use std::rc::Rc;
//! use craft_web_api::methods::Methods;
//! use craft_web_api::page::{PageApi, Request};
//!
//! // The control channel: send a ControlRequest, await its reply object.
//! let request: Request = Rc::new(move |method: String, params: serde_json::Value| {
//!     let (req, rx) = ControlRequest::new(method, params);
//!     control_tx.send(req); // the same channel the desktop control server uses
//!     Box::pin(receive_reply(rx)) // resolves with {ok, result} / {ok: false, error}
//! });
//! let page = Rc::new(PageApi::new("vectorcraft", Methods::default(), request));
//! page.install()?; // window.vectorcraft
//! ```

pub mod methods;

#[cfg(target_arch = "wasm32")]
pub mod page;
