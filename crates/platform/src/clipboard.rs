//! The system clipboard for copy and paste of text, rich text, images and files (N10.01).
//!
//! - [`Clipboard`]: the one write and read path every copy and paste command goes through. It
//!   applies the size limits ([`ClipboardLimits`]), keeps input injected by automation from
//!   replacing what the user copied ([`Clipboard::set_synthetic`]), and remembers what it wrote
//!   last so the app can tell its own copy from one made in another app
//!   ([`Clipboard::holds_ours`]).
//! - [`ClipboardBackend`]: where the bytes go. [`SystemClipboard`] is the OS clipboard on the
//!   desktop (arboard, opened on first use); [`EventClipboard`] is the browser's, fed from paste
//!   events; tests plug in their own.
//! - [`ClipboardProbe`]: asks a second clipboard handle on a worker thread whether Paste has
//!   something to take, so a slow clipboard owner never stalls a frame.
//!
//! No egui.

mod backend;
mod convert;
mod event;
mod facade;
mod probe;
mod types;

pub use backend::*;
pub(crate) use convert::*;
pub use event::*;
pub use facade::*;
pub use probe::*;
pub use types::*;

// The probe's backend lives on its worker.
const _: () = {
    const fn sendable<T: Send>() {}
    sendable::<EventClipboard>();
};
