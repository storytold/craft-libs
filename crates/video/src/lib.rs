#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room pure-Rust video codecs for the Craft apps, one feature per codec. Input and output
//! are byte slices and planar sample buffers (no filesystem access of its own); builds for
//! `wasm32-unknown-unknown`.
//!
//! | Feature | Module | Codec |
//! |---|---|---|
//! | `dnx` | [`dnx`] | Avid DNxHD / DNxHR (SMPTE ST 2019-1): decoder and encoder |
//! | `apv` | [`apv`] | APV (RFC 9924): decoder (`apv-decode`) and encoder (`apv-encode`) |
//! | `vp8` | [`vp8`] | VP8 (RFC 6386): decoder |
//! | `prores` | [`prores`] | Apple ProRes (SMPTE RDD 36): decoder and encoder |
//! | `h264` | [`h264`] | ITU-T H.264 / AVC: encoder |
//! | `hevc` | [`hevc`] | ITU-T H.265 / HEVC: decoder (`hevc-decode`) and encoder (`hevc-encode`) |
//! | `vp9` | [`vp9`] | VP9: decoder (`vp9-decode`) and encoder (`vp9-encode`) |
//! | `av1` | [`av1`] | AV1 (AOMedia Video 1): encoder |
//!
//! No codec is on by default. Feature `threads` spreads the work of the codecs that support it
//! over rayon worker threads.
//!
//! The decoders bound the pictures they hold with a memory budget ([`budget`]).
//!
//! Containers and bit-level I/O are in `craft-media`; the codecs here share its `bitstream`
//! module and describe their streams with `craft_media::CodecParams`.

pub mod budget;

#[cfg(any(feature = "apv-decode", feature = "apv-encode"))]
pub mod apv;

#[cfg(feature = "dnx")]
pub mod dnx;

#[cfg(feature = "vp8")]
pub mod vp8;

#[cfg(feature = "prores")]
pub mod prores;

#[cfg(feature = "h264")]
pub mod h264;

#[cfg(any(feature = "hevc-decode", feature = "hevc-encode"))]
pub mod hevc;

#[cfg(any(feature = "vp9-decode", feature = "vp9-encode"))]
pub mod vp9;

#[cfg(feature = "av1")]
pub mod av1;
