//! `NativeWindow`: the operating system's window behind an app window, for what egui has no command for: the
//! Windows hit test of an app-drawn frame (snap layouts, native move and resize) and a window's opacity.
//!
//! The hit test is answered by [`Hits`], which the app side implements from the exclusive parts of its last pass.
//! The pure part (which zone is which `WM_NCHITTEST` value) is here and runs everywhere; the Win32 calls are in
//! `win32.rs`, which calls craft-win32 for the `unsafe` ones.

use std::sync::Arc;

/// An edge or corner of a window that a resize starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Edge {
    #[allow(missing_docs)]
    Top,
    #[allow(missing_docs)]
    Bottom,
    #[allow(missing_docs)]
    Left,
    #[allow(missing_docs)]
    Right,
    #[allow(missing_docs)]
    TopLeft,
    #[allow(missing_docs)]
    TopRight,
    #[allow(missing_docs)]
    BottomLeft,
    #[allow(missing_docs)]
    BottomRight,
}

/// What the app draws at a point of the window frame, for the operating system's hit test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Zone {
    /// The area that drags the window.
    Move,
    /// An edge or corner that resizes the window.
    Edge(Edge),
    /// A caption button.
    Button(crate::Button),
}

/// What the pointer does at a caption button the operating system owns the pointer over (the maximize button, whose
/// hit test answers `HTMAXBUTTON`: the pointer lies in the non-client area, so the app gets no client pointer there).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum CaptionEvent {
    /// The pointer is over the button.
    Hover(crate::Button),
    /// The pointer left the button without a press ending on it.
    Leave,
    /// The primary button went down on the button.
    Press(crate::Button),
    /// The primary button went up on the button: a click when it went down there.
    Release(crate::Button),
    /// The press ended away from the button.
    Cancel,
    /// The primary button went down twice on the drag area, at this client pixel. The operating system's own
    /// handling (it would maximize the window) is replaced by the app's.
    BandDouble([f32; 2]),
    /// The secondary button went up on the drag area, at this client pixel. The system menu does not open.
    BandMenu([f32; 2]),
}

/// What a window procedure asks the app side.
pub trait Hits: Send + Sync {
    /// What the app draws at `pixel`, in physical pixels from the top left of the client area; `None` leaves the hit
    /// test to the window's content.
    fn zone_at(&self, pixel: [f32; 2]) -> Option<Zone>;

    /// A caption button's pointer event, from any thread the window procedure runs on.
    fn caption(&self, event: CaptionEvent);
}

/// The `WM_NCHITTEST` result for `zone`, or `None` for the default hit test. Only the maximize button is the
/// operating system's (snap layouts show over it); the other buttons are answered in the client area.
#[cfg(windows)]
pub(crate) fn hit_code(zone: Option<Zone>) -> Option<u16> {
    Some(match zone? {
        Zone::Move => 2,
        Zone::Edge(Edge::Left) => 10,
        Zone::Edge(Edge::Right) => 11,
        Zone::Edge(Edge::Top) => 12,
        Zone::Edge(Edge::TopLeft) => 13,
        Zone::Edge(Edge::TopRight) => 14,
        Zone::Edge(Edge::Bottom) => 15,
        Zone::Edge(Edge::BottomLeft) => 16,
        Zone::Edge(Edge::BottomRight) => 17,
        Zone::Button(crate::Button::Maximize) => 9,
        Zone::Button(_) => return None,
    })
}

/// The operating system window of an app window.
#[derive(Clone, Debug)]
pub struct NativeWindow {
    #[cfg(windows)]
    hwnd: crate::win32::Hwnd,
    #[cfg(not(windows))]
    title: String,
}

impl NativeWindow {
    /// The window titled `title`. On Windows, among several with that title, only the one whose client area starts
    /// within a pixel or two of `client_origin` (physical screen pixels) and that is not routed yet, and none when
    /// that is not exactly one window (the caller tries again later); on macOS AppKit finds windows by title,
    /// so two windows with one title share what is set on them. `None` where the platform has no such window, an
    /// empty title, and platforms that do not support it.
    pub fn titled(title: &str, client_origin: Option<[i32; 2]>) -> Option<Self> {
        if title.is_empty() {
            return None;
        }
        #[cfg(windows)]
        {
            let hwnd = crate::win32::find(title, client_origin)?;
            Some(NativeWindow { hwnd })
        }
        #[cfg(not(windows))]
        {
            let _ = client_origin;
            cfg!(all(feature = "appkit", target_os = "macos")).then(|| NativeWindow { title: title.to_owned() })
        }
    }

    /// The window `handle` names (the root window's handle from eframe's `Frame`). `None` for a handle of another
    /// platform.
    #[cfg(windows)]
    pub fn of_handle(handle: &impl raw_window_handle::HasWindowHandle) -> Option<Self> {
        let raw = handle.window_handle().ok()?.as_raw();
        let raw_window_handle::RawWindowHandle::Win32(win32) = raw else { return None };
        Some(NativeWindow { hwnd: crate::win32::Hwnd::new(win32.hwnd.get()) })
    }

    /// Sets the window's opacity, 1 being opaque. Returns whether the platform applied it: a layered window on
    /// Windows, `NSWindow.alphaValue` on macOS, nothing elsewhere.
    pub fn set_opacity(&self, opacity: f32) -> bool {
        let opacity = opacity.clamp(0.0, 1.0);
        #[cfg(windows)]
        {
            crate::win32::set_opacity(self.hwnd, opacity)
        }
        #[cfg(not(windows))]
        {
            crate::mac::set_opacity(&self.title, opacity)
        }
    }

    /// Makes the window procedure answer the hit test from `hits`. Returns whether the platform routes it: Windows
    /// only. The routing ends with the window.
    pub fn route_hits(&self, hits: Arc<dyn Hits>) -> bool {
        #[cfg(windows)]
        {
            crate::win32::route(self.hwnd, hits)
        }
        #[cfg(not(windows))]
        {
            let _ = hits;
            false
        }
    }
}
