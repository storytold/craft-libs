#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Media containers and bit-level I/O for the Craft apps.
//!
//! - `bitstream` (feature `bitstream`): MSB-first bit reader and writer, Exp-Golomb codes,
//!   emulation-prevention escaping and NAL unit splitting.
//! - `isobmff` (feature `isobmff`, enables `bitstream`): a clean-room ISO BMFF (MP4) and
//!   QuickTime (MOV) demuxer and muxer, and a box reader for other ISO BMFF formats.
//! - `matroska` (feature `matroska`): a clean-room Matroska (MKV) and WebM demuxer and muxer.
//!
//! `bitstream` and `isobmff` are on by default; `matroska` is opt-in. A crate that needs only bit
//! I/O uses `default-features = false, features = ["bitstream"]`. Codecs stay in the apps.

#[cfg(feature = "bitstream")]
pub mod bitstream;
#[cfg(feature = "isobmff")]
pub mod isobmff;
#[cfg(feature = "matroska")]
pub mod matroska;
