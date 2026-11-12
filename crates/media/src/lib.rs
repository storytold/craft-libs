#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Media containers and bit-level I/O for the Craft apps. Codecs are in other crates
//! (`craft-audio`; video later), which depend on this one.
//!
//! - `bitstream` (feature `bitstream`): MSB-first bit reader and writer, Exp-Golomb codes,
//!   emulation-prevention escaping and NAL unit splitting.
//! - `isobmff` (feature `isobmff`, enables `bitstream`): a clean-room ISO BMFF (MP4) and
//!   QuickTime (MOV) demuxer and muxer, and a box reader for other ISO BMFF formats.
//! - `matroska` (feature `matroska`): a clean-room Matroska (MKV) and WebM demuxer and muxer.
//! - `ogg` (feature `ogg`): a clean-room Ogg demuxer with Ogg Opus and Ogg Vorbis stream mapping.
//! - `avi` (feature `avi`): a clean-room AVI (RIFF) and OpenDML demuxer.
//! - `mxf` (feature `mxf`, enables `chrono`): a clean-room MXF demuxer and writer (OP1a, OP-Atom).
//! - `mpegts` (feature `mpegts`): a clean-room MPEG-2 Systems demuxer (transport streams and
//!   program streams).
//! - [`Container`] and [`CodecId`] (no feature): the identifiers the codec crates report a file's
//!   container and codec with.
//!
//! `bitstream` and `isobmff` are on by default; `matroska`, `ogg`, `avi`, `mxf` and `mpegts` are opt-in. A crate that needs
//! only bit I/O uses `default-features = false, features = ["bitstream"]`.

#[cfg(feature = "avi")]
pub mod avi;
#[cfg(feature = "bitstream")]
pub mod bitstream;
mod id;
#[cfg(feature = "isobmff")]
pub mod isobmff;
#[cfg(feature = "matroska")]
pub mod matroska;
#[cfg(feature = "mpegts")]
pub mod mpegts;
#[cfg(feature = "mxf")]
pub mod mxf;
#[cfg(feature = "ogg")]
pub mod ogg;
pub mod source;

pub use id::{CodecId, CodecParams, Container, PcmFormat, xiph_lace};
pub use source::{ByteSource, ReadSeekSource};
