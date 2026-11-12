#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! OpenEXR handling shared by the Craft apps.
//!
//! This first part is the naming layer every reader needs once it has a file's channel list and
//! header attributes, whichever decoder produced them:
//!
//! - [`channels`]: OpenEXR layer and channel names (`diffuse.R`, `ViewLayer.Combined.R`), the
//!   channels that make a colour picture, and the channels of a layer to show as red, green, blue
//!   and alpha.
//! - [`cryptomatte`]: Cryptomatte (specification 1.2) IDs, metadata keys, manifests and rank
//!   channel names.
//!
//! Those take names and attribute text and depend on no decoder. With the `decode` feature:
//!
//! - [`decode`]: bounded part listing, flat decoding (scan lines and tiles, every compression the
//!   `exr` crate reads) and writing, on top of the `exr` crate.
//! - [`deep`]: deep scan lines and tiles (NONE, RLE, ZIPS) and their flattening.
//! - [`stream`]: one flat scan-line part read block by block, for pictures larger than memory.
//! - [`cryptomatte_samples`]: a file's Cryptomatte layers and their per-pixel (ID, coverage) ranks.

pub mod channels;
pub mod cryptomatte;
#[cfg(feature = "decode")]
pub mod cryptomatte_samples;
#[cfg(feature = "decode")]
pub mod decode;
#[cfg(all(test, feature = "decode"))]
mod decode_tests;
#[cfg(feature = "decode")]
pub mod deep;
#[cfg(all(test, feature = "decode"))]
mod deep_tests;
#[cfg(feature = "decode")]
pub mod stream;
