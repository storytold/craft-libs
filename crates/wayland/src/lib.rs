//! The Wayland surface call that needs `unsafe`, behind a safe API: unmapping a window while it and its connection
//! stay open. Linux only; the crate is empty elsewhere.
//!
//! This is one of the crates that may allow `unsafe`; the design-lint rule `contract/unsafe-allow` names it.
#![warn(missing_docs)]
#![cfg(target_os = "linux")]

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

/// Attaches no buffer to the window's Wayland surface and commits it, which unmaps the window while it and its
/// connection stay open. `Ok(false)` when the window is not a Wayland window; `Err` when the compositor cannot be
/// reached.
pub fn unmap_window(window: &(impl HasWindowHandle + HasDisplayHandle)) -> std::io::Result<bool> {
    let _ = window;
    todo!("fl.F39")
}
