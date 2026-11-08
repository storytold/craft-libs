#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room pure-Rust video codecs for the Craft apps, one feature per codec. Input and output
//! are byte slices and planar sample buffers (no filesystem access of its own); builds for
//! `wasm32-unknown-unknown`.
//!
//! | Feature | Module | Codec |
//! |---|---|---|
//! | `dnx` | [`dnx`] | Avid DNxHD / DNxHR (SMPTE ST 2019-1): decoder and encoder |
//! | `vp8` | [`vp8`] | VP8 (RFC 6386): decoder |
//!
//! No codec is on by default. Feature `threads` spreads the work of the codecs that support it
//! over rayon worker threads.
//!
//! Containers and bit-level I/O are in `craft-media`; the codecs here share its `bitstream`
//! module and describe their streams with `craft_media::CodecParams`.

#[cfg(feature = "dnx")]
pub mod dnx;

#[cfg(feature = "vp8")]
pub mod vp8;
