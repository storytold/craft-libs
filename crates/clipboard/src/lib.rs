#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Native clipboard MIME transfers on the focused app's existing Wayland connection.
//! No data-control permission, hidden window or X server is required.
#[cfg(target_os = "linux")]
mod source;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::Clipboard;

#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
pub use portal::{FILE_TRANSFER_MIME, file_transfer_key, retrieve_files};
