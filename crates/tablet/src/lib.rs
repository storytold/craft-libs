#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Pen tablet pressure on X11 and Xwayland, which winit reports as a plain mouse.
//!
//! [`spawn`] opens a second, pure-Rust X connection (x11rb's `RustConnection`, no libxcb), selects
//! XInput2 raw events on the root window and reports the pen's pressure from a thread of its own.
//! Raw events reach every client that asks, also while the window holds a drag's pointer grab, but
//! don't say which window the pen is over: the pressure is meant to be read only while one of the
//! app's views receives pointer events, and any other pointer moving reports a mouse.
//!
//! A native Wayland window gets no tablet input this way; an app should not start the reader for
//! one. [`xi`] is the pure mapping from XInput2 valuators to pressure and builds everywhere. The
//! reader itself is Linux only: elsewhere [`spawn`] returns [`Error::Unsupported`].

#[cfg(target_os = "linux")]
mod x11;
pub mod xi;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

/// The name of the input interface the reader uses, for an app's About box or diagnostics.
pub const INTERFACE: &str = "XInput2";

/// Why the reader did not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not Linux: there is no X server to read.
    Unsupported,
    /// No X server, no XInput 2.1, or the reader thread could not start.
    Unavailable(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("tablet pressure through XInput2 is read on Linux only"),
            Self::Unavailable(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {}

/// Starts reading pen pressure on `$DISPLAY` in a thread named `thread_name`.
///
/// `on_update` gets `Some(pressure)` (0..=1) for each raw event from a pen and `None` when another
/// pointer moves. `on_closed` runs once, on the reader thread, when the X connection closes. Without
/// an X server or XInput 2.1 this returns an error and starts nothing; a pen then keeps working as
/// a mouse.
pub fn spawn(thread_name: &str, on_update: impl FnMut(Option<f32>) + Send + 'static, on_closed: impl FnOnce() + Send + 'static) -> Result<(), Error> {
    #[cfg(target_os = "linux")]
    {
        x11::spawn(thread_name, on_update, on_closed)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (thread_name, on_update, on_closed);
        Err(Error::Unsupported)
    }
}
