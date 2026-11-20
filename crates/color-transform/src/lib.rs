//! Color transforms for the Craft apps.
//!
//! - [`Mat3`], CIE [`Xy`] chromaticities and RGB spaces defined by primaries and a white point
//!   ([`RgbSpace`]), with Bradford chromatic adaptation ([`bradford`]).
//! - White balance by correlated color temperature and tint ([`cct`]).
//! - OkLab and OkLCh ([`oklab`]).
//! - Monotone tone curves and 1D lookup tables ([`curve`]).
//! - PQ tone mapping with the reference EETF of Report ITU-R BT.2408 Annex 5 ([`tonemap`]).
//!
//! Frame color tags, Y'CbCr matrices and the transfer functions (sRGB, PQ, HLG) are in
//! `craft-color`; this crate builds on them. Every function that runs per pixel works on `f32`
//! without allocating, and builds for `wasm32-unknown-unknown`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cct;
pub mod curve;
pub mod oklab;
mod space;
pub mod tonemap;

pub use space::{BRADFORD, D50, D65, DISPLAY_P3, Mat3, REC601_525, REC601_625, REC2020, RgbSpace, SRGB, Xy, bradford, bradford_xyz};
