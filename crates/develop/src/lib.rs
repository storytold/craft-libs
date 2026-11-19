//! The raw develop pipeline, CPU path: the shared pipeline of craft-libs#587, moved from LightCraft one module at a
//! time. See the README for what is here so far.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod color;
pub mod curve;
pub mod inpaint;
pub mod lut;
