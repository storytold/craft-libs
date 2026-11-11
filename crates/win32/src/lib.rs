//! The Win32 calls that need `unsafe`, behind a safe API: subclassing a window's procedure with a handler, and
//! making a window handle from its raw value. Windows only; the crate is empty elsewhere.
//!
//! This is the only crate (besides the benchmark's allocator) that allows `unsafe`; the design-lint rule
//! `contract/unsafe-allow` names it. Every block says why it is sound.
#![cfg(windows)]
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{LazyLock, Mutex, PoisonError};
use winsafe::{self as w, co, msg};

/// Tells this crate's subclass from another one on the same window.
const SUBCLASS_ID: usize = 0x4352_5733;

/// Answers a message of a subclassed window: `Some(result)` ends it there, `None` passes it to the window's own
/// procedure. It runs on the thread that owns the window. A destroyed window sends it `WM_NCDESTROY` last.
pub type Handler = fn(window: &w::HWND, message: co::WM, wparam: usize, lparam: isize) -> Option<isize>;

static HANDLERS: LazyLock<Mutex<HashMap<isize, Handler>>> = LazyLock::new(Default::default);

/// The window handle with the raw value `raw`, from `HWND::ptr() as isize` or a platform handle. A value that names
/// no live window only makes calls on the handle fail.
pub fn handle(raw: isize) -> w::HWND {
    // SAFETY: `HWND` has no destructor and nothing here dereferences it; the system validates the handle in every
    // call made with it, as for any stale window handle.
    unsafe { w::HWND::from_ptr(raw as *mut c_void) }
}

/// The raw value of `window`.
pub fn raw(window: &w::HWND) -> isize {
    window.ptr() as isize
}

/// Puts a subclass on the window `raw` that offers every message to `handler` first. Returns whether the window is
/// subclassed (a window subclassed before keeps its first handler). The subclass ends with the window. The window
/// must belong to the calling thread.
pub fn subclass(raw: isize, handler: Handler) -> bool {
    if handlers(|handlers| handlers.contains_key(&raw)) {
        return true;
    }
    let window = handle(raw);
    // SAFETY: `procedure` is a plain function with the `SUBCLASSPROC` signature that outlives every window, and the
    // reference data is unused. Comctl32 calls it on the thread that owns the window; for another thread's window
    // `SetWindowSubclass` fails.
    let set = unsafe { window.SetWindowSubclass(procedure, SUBCLASS_ID, 0) };
    if set.is_err() {
        return false;
    }
    handlers(|handlers| handlers.insert(raw, handler));
    true
}

fn handlers<R>(f: impl FnOnce(&mut HashMap<isize, Handler>) -> R) -> R {
    f(&mut HANDLERS.lock().unwrap_or_else(PoisonError::into_inner))
}

extern "system" fn procedure(window: w::HWND, message: co::WM, wparam: usize, lparam: isize, _id: usize, _data: usize) -> isize {
    let key = raw(&window);
    let handler = handlers(|handlers| handlers.get(&key).copied());
    if let Some(answer) = handler.and_then(|handler| handler(&window, message, wparam, lparam)) {
        return answer;
    }
    // SAFETY: the message and its parameters are the ones the system delivered to this procedure, passed on
    // unchanged to the next procedure of the chain.
    let result = unsafe { window.DefSubclassProc(msg::Wm::new(message, wparam, lparam)) };
    if message == co::WM::NCDESTROY {
        handlers(|handlers| handlers.remove(&key));
        // The window ends with this message, so a failed removal leaves nothing to clean up.
        let _ = window.RemoveWindowSubclass(procedure, SUBCLASS_ID);
    }
    result
}
