//! The raw develop pipeline, CPU path: the shared pipeline of craft-libs#587, moved from LightCraft one module at a
//! time. See the README for what is here so far.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod auto;
pub mod color;
pub mod colorops;
pub mod crystal;
pub mod curve;
pub mod finish;
pub mod inpaint;
pub mod light_v3;
// generated (`light_v3_params.rs`), kept in its own compact layout
#[rustfmt::skip]
pub mod light_v3_params;
pub mod local;
pub mod local_tone;
pub mod lut;
pub mod nrc_joint;
#[cfg(test)]
mod nrc_joint_reference;
pub mod profiles;
pub mod raster;
pub mod sensor;
pub mod tone;
