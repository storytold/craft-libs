//! Holding a browser tab open while work is unsaved.
//!
//! A tab has no close request an app can answer: the browser asks the user itself when the page's
//! `beforeunload` listener calls `prevent_default`. So the hold is the browser's own prompt, fed
//! each frame with whether work is at risk.

use std::cell::{Cell, OnceCell};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::BeforeUnloadEvent;

thread_local! {
    /// What the last [`hold_leave`] said. The listener runs on the page's one thread, outside any
    /// frame, so it reads the last value the app gave.
    static UNSAVED: Cell<bool> = const { Cell::new(false) };
    /// The listener, kept alive for the page's life once the window has it.
    static LISTENER: OnceCell<Closure<dyn FnMut(BeforeUnloadEvent)>> = const { OnceCell::new() };
}

/// Keeps one `beforeunload` listener that calls `prevent_default` while the last call said
/// `unsaved`. Call it each frame with whether work is at risk.
///
/// The first call adds the listener; a page without a window (a worker) or one that refuses the
/// listener gets none, and a later call tries again.
pub fn hold_leave(unsaved: bool) {
    UNSAVED.with(|cell| cell.set(unsaved));
    LISTENER.with(|listener| {
        if listener.get().is_some() {
            return;
        }
        let Some(window) = web_sys::window() else { return };
        let handler = Closure::<dyn FnMut(BeforeUnloadEvent)>::new(|event: BeforeUnloadEvent| {
            if UNSAVED.with(Cell::get) {
                event.prevent_default();
            }
        });
        if window.add_event_listener_with_callback("beforeunload", handler.as_ref().unchecked_ref()).is_ok() {
            let _ = listener.set(handler);
        }
    });
}
