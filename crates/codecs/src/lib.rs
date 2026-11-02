#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Storytold ArtCraft shared video codecs library.
//!
//! Centralizes high-performance pure-Rust video codecs:
//! - H.264 / AVC video encoding (`craft-h264enc`)
//! - Apple ProRes video encoding and decoding (`craft-prores`)
//! - Avid DNxHD / DNxHR SMPTE ST 2019-1 encoding and decoding (`craft-dnx`)
//! - HEVC / H.265 video encoding (`craft-hevcenc`)
//! - AV1 video encoding (`craft-av1enc`)
//! - VP9 video encoding (`craft-vp9enc`)
//! - VP8 video decoding (`craft-vp8`)

#[cfg(feature = "h264")]
pub use craft_h264enc as h264;

#[cfg(feature = "prores")]
pub use craft_prores as prores;

#[cfg(feature = "dnx")]
pub use craft_dnx as dnx;

#[cfg(feature = "hevc")]
pub use craft_hevcenc as hevc;

#[cfg(feature = "av1")]
pub use craft_av1enc as av1;

#[cfg(feature = "vp9")]
pub use craft_vp9enc as vp9;

#[cfg(feature = "vp8")]
pub use craft_vp8 as vp8;
