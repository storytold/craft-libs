#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room pure-Rust video codecs for the Craft apps, one feature per codec. Input and output
//! are byte slices and planar sample buffers (no filesystem access of its own); builds for
//! `wasm32-unknown-unknown`.
//!
//! | Feature | Module | Codec |
//! |---|---|---|
//! | `dnx` | `dnx` | Avid DNxHD / DNxHR (SMPTE ST 2019-1): decoder and encoder |
//! | `apv` | `apv` | APV (RFC 9924): decoder (`apv-decode`) and encoder (`apv-encode`) |
//! | `vp8` | `vp8` | VP8 (RFC 6386): decoder (`vp8-decode`) and key-frame encoder (`vp8-encode`) |
//! | `mpeg2` | `mpeg2` | MPEG-2 video (ITU-T H.262) and MPEG-1 video: decoder (`mpeg2-decode`) and MPEG-2 I/P/B encoder with CBR / VBR (`mpeg2-encode`) |
//! | `mpeg4` | `mpeg4` | MPEG-4 Part 2 video (ISO/IEC 14496-2, Xvid/DivX): Simple profile encoder (`mpeg4-encode`) |
//! | `ffv1` | `ffv1` | FFV1 lossless video (RFC 9043): version 3, range coder, decoder and encoder |
//! | `rawvideo` | `rawvideo` | Uncompressed packed pictures: UYVY, YUYV, YVYU, v210, r210, R10k |
//! | `hap` | `hap` | Vidvox Hap, Hap Alpha, Hap Q, Hap Q Alpha, Hap Alpha-Only: decoder and encoder |
//! | `mjpeg` | `mjpeg` | Motion JPEG: frame-level decoder and baseline encoder over `craft-jpeg` |
//! | `prores` | `prores` | Apple ProRes (SMPTE RDD 36): decoder and encoder |
//! | `h264` | `h264` | ITU-T H.264 / AVC: decoder (`h264-decode`) and encoder (`h264-encode`) |
//! | `hevc` | `hevc` | ITU-T H.265 / HEVC: decoder (`hevc-decode`) and encoder (`hevc-encode`) |
//! | `vp9` | `vp9` | VP9: decoder (`vp9-decode`) and encoder (`vp9-encode`) |
//! | `av1` | `av1` | AV1 (AOMedia Video 1): decoder (`av1-decode`) and encoder (`av1-encode`) |
//! | `vmd` | `vmd` | Sierra VMD video (8-bit paletted): decoder (`vmd-decode`) |
//! | `bink` | `bink` | Bink video (Bink 1, revisions `f` and `i`): decoder (`bink-decode`) |
//!
//! No codec is on by default. Feature `threads` spreads the work of the codecs that support it
//! over rayon worker threads.
//!
//! The decoders bound the pictures they hold with a memory budget ([`budget`]).
//!
//! Each decoder returns its own picture type. [`frame::VideoFrame`] is the codec-agnostic form
//! they all convert into (`VideoFrame::try_from`).
//!
//! `TrackDecoder` picks the decoder for a track from its `craft_media::CodecParams` and returns
//! `VideoFrame`s, for H.264, HEVC, VP9, AV1, MPEG-2, ProRes, DNx, APV, FFV1 and VP8
//! (built with any of their decoder features).
//!
//! Containers and bit-level I/O are in `craft-media`; the codecs here share its `bitstream`
//! module and describe their streams with `craft_media::CodecParams`.

pub mod budget;
mod error;
pub mod frame;

pub use error::VideoError;

#[cfg(any(
    feature = "h264-decode",
    feature = "hevc-decode",
    feature = "vp9-decode",
    feature = "av1-decode",
    feature = "mpeg2-decode",
    feature = "prores",
    feature = "dnx",
    feature = "apv-decode",
    feature = "ffv1",
    feature = "vp8-decode",
    feature = "mjpeg"
))]
mod track;
#[cfg(any(
    feature = "h264-decode",
    feature = "hevc-decode",
    feature = "vp9-decode",
    feature = "av1-decode",
    feature = "mpeg2-decode",
    feature = "prores",
    feature = "dnx",
    feature = "apv-decode",
    feature = "ffv1",
    feature = "vp8-decode",
    feature = "mjpeg"
))]
pub use track::{TrackDecoder, TrackDecoderOptions};

#[cfg(any(feature = "apv-decode", feature = "apv-encode"))]
pub mod apv;

#[cfg(feature = "bink-decode")]
pub mod bink;

#[cfg(feature = "dnx")]
pub mod dnx;

#[cfg(any(feature = "vp8-decode", feature = "vp8-encode"))]
pub mod vp8;

#[cfg(feature = "mpeg2-decode")]
pub mod mpeg2;

#[cfg(feature = "mpeg4-encode")]
pub mod mpeg4;

#[cfg(feature = "ffv1")]
pub mod ffv1;

#[cfg(feature = "hap")]
pub mod hap;
#[cfg(feature = "rawvideo")]
pub mod rawvideo;

#[cfg(feature = "mjpeg")]
pub mod mjpeg;

#[cfg(feature = "prores")]
pub mod prores;

#[cfg(any(feature = "h264-decode", feature = "h264-encode"))]
pub mod h264;

#[cfg(any(feature = "hevc-decode", feature = "hevc-encode"))]
pub mod hevc;

#[cfg(any(feature = "vp9-decode", feature = "vp9-encode"))]
pub mod vp9;

#[cfg(any(feature = "av1-decode", feature = "av1-encode"))]
pub mod av1;

/// Uncompressed AVI and QuickTime pixel layouts.
pub mod raw_video;

/// Bounded Screen Video key-frame decoding (FLV codec 3).
#[cfg(feature = "screen-video")]
pub mod screen_video;

/// Theora video (Xiph.Org specification): decoder and encoder.
#[cfg(any(feature = "theora-decode", feature = "theora-encode"))]
pub mod theora;

#[cfg(feature = "vmd-decode")]
pub mod vmd;
