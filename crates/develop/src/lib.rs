//! The raw develop pipeline (CPU path).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod color;
pub mod inpaint;
pub mod preview;
pub mod raster;
pub mod sensor;
