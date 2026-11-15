//! PostScript and EPS for the Craft apps.
//!
//! - [`read`] (feature `read`): runs an EPS and gives what it paints as a one-page vector PDF,
//!   for apps that place EPS files as graphics.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(feature = "read")]
pub mod read;
