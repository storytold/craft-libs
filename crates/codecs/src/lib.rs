#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Storytold ArtCraft shared media codecs library.
//!
//! Centralizes high-performance pure-Rust media codecs across video and audio:
//! - H.264 / AVC video encoding (`craft-h264enc`)
//! - Apple ProRes video encoding and decoding (`craft-prores`)
//! - HEVC / H.265 video encoding (`craft-hevcenc`)
//! - AV1 video encoding (`craft-av1enc`)
//! - VP9 video encoding (`craft-vp9enc`)
//! - AAC-LC audio encoding and decoding (`craft-aac`)
//! - Universal audio decoding & FLAC/AIFF encoding (`craft-audio-io`)

#[cfg(feature = "h264")]
pub use craft_h264enc as h264;

#[cfg(feature = "prores")]
pub use craft_prores as prores;

#[cfg(feature = "hevc")]
pub use craft_hevcenc as hevc;

#[cfg(feature = "av1")]
pub use craft_av1enc as av1;

#[cfg(feature = "vp9")]
pub use craft_vp9enc as vp9;

#[cfg(feature = "aac")]
pub use craft_aac as aac;

#[cfg(feature = "audio-io")]
pub use craft_audio_io as audio_io;
