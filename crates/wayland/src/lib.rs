//! The Wayland surface call that needs `unsafe`, behind a safe API: unmapping a window while it and its connection
//! stay open. Linux only; the crate is empty elsewhere.
//!
//! This is one of the crates that may allow `unsafe`; the design-lint rule `contract/unsafe-allow` names it. Every
//! block says why it is sound.
#![warn(missing_docs)]
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::client::WaylandError;
use wayland_backend::sys::client::{Backend, ObjectId};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};

/// Attaches no buffer to the window's Wayland surface and commits it, which unmaps the window while it and its
/// connection stay open. `Ok(false)` when the window is not a Wayland window; `Err` when the compositor cannot be
/// reached.
pub fn unmap_window(window: &(impl HasWindowHandle + HasDisplayHandle)) -> std::io::Result<bool> {
    let (Ok(window_handle), Ok(display_handle)) = (window.window_handle(), window.display_handle()) else { return Ok(false) };
    let (RawWindowHandle::Wayland(handle), RawDisplayHandle::Wayland(display)) = (window_handle.as_raw(), display_handle.as_raw()) else {
        return Ok(false);
    };
    // SAFETY: the display is the window's open `wl_display`, which outlives the borrowed `window` and so this call; a
    // backend made from a foreign display does not close it when dropped.
    let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
    let connection = Connection::from_backend(backend);
    // SAFETY: the pointer is the window's live `wl_surface` proxy on that display, alive while `window` is borrowed.
    let Ok(id) = (unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }) else { return Ok(false) };
    let Ok(surface) = WlSurface::from_id(&connection, id) else { return Ok(false) };
    surface.attach(None, 0, 0);
    surface.commit();
    connection.flush().map_err(|error| match error {
        WaylandError::Io(error) => error,
        other => std::io::Error::other(other),
    })?;
    Ok(true)
}
