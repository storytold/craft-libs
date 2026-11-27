//! Clean-room pure-Rust VP8, implemented from RFC 6386 ("VP8 Data Format and Decoding Guide").
//!
//! | Feature | Module | What |
//! |---|---|---|
//! | `vp8-decode` | this module | Decoder: sections 1-19, key and inter frames ([`Decoder`]) |
//! | `vp8-encode` | this module | Encoder: key frames, as the lossy picture of a WebP image ([`encode_rgba`]) |
//!
//! Feature `vp8` turns on both. The decoder:
//!
//! Supported: the whole format — key frames and inter frames, all intra modes (16x16, chroma,
//! the ten 4x4 subblock modes), inter prediction from the last, golden and altref frames with
//! sign bias, NEAREST / NEAR / ZERO / NEW / SPLIT motion vectors, six-tap and bilinear
//! interpolation (versions 0-3, full-pixel chroma), segmentation, the normal and simple loop
//! filters with mode / reference deltas, multiple token partitions, probability persistence
//! (`refresh_entropy_probs`), golden / altref copies and hidden (altref) frames.
//!
//! ```no_run
//! let mut dec = craft_video::vp8::Decoder::new();
//! let frame: Vec<u8> = Vec::new(); // one IVF / WebM frame
//! if let Some(pic) = dec.decode(&frame, 0).unwrap() {
//!     println!("{}x{}", pic.width, pic.height);
//! }
//! ```

#![allow(clippy::needless_range_loop)]

#[cfg(feature = "vp8-decode")]
mod boolcoder;
#[cfg(feature = "vp8-decode")]
mod decoder;
#[cfg(feature = "vp8-decode")]
mod error;
#[cfg(feature = "vp8-decode")]
mod header;
#[cfg(feature = "vp8-decode")]
mod loopfilter;
#[cfg(feature = "vp8-decode")]
mod predict;
#[cfg(feature = "vp8-decode")]
#[rustfmt::skip]
mod spec_tables;
#[cfg(feature = "vp8-decode")]
mod tables;
#[cfg(feature = "vp8-decode")]
mod transform;

#[cfg(feature = "vp8-decode")]
pub use decoder::{DecodeStats, Decoder};
#[cfg(feature = "vp8-decode")]
pub use error::{Error, Result};

#[cfg(feature = "vp8-decode")]
/// A decoded picture: 8-bit 4:2:0, planes tight (stride = width).
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub chroma_width: u32,
    pub chroma_height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    /// `pts` passed to [`Decoder::decode`] with the frame.
    pub pts: i64,
    pub key: bool,
    /// `color_space` of the last key frame (0: BT.601 YUV, 1: reserved).
    pub color_space: u8,
}

#[cfg(feature = "vp8-decode")]
/// Stream parameters read from a key frame's uncompressed chunk (see [`keyframe_info`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyframeInfo {
    pub width: u32,
    pub height: u32,
    /// Upscaling codes (9.1), informational only.
    pub horizontal_scale: u8,
    pub vertical_scale: u8,
    pub version: u8,
}

#[cfg(feature = "vp8-decode")]
/// If `frame` is a key frame (a random access point), its parameters.
pub fn keyframe_info(frame: &[u8]) -> Option<KeyframeInfo> {
    let tag = header::FrameTag::parse(frame).ok()?;
    (tag.key_frame && tag.width > 0 && tag.height > 0).then_some(KeyframeInfo {
        width: u32::from(tag.width),
        height: u32::from(tag.height),
        horizontal_scale: tag.horizontal_scale,
        vertical_scale: tag.vertical_scale,
        version: tag.version,
    })
}

#[cfg(feature = "vp8-decode")]
/// Whether `frame` is a key frame (decoding can start there).
pub fn is_keyframe(frame: &[u8]) -> bool {
    keyframe_info(frame).is_some()
}

#[cfg(feature = "vp8-encode")]
mod encode;
#[cfg(feature = "vp8-encode")]
pub use encode::*;
