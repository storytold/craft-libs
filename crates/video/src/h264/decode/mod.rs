//! Clean-room pure-Rust H.264 / AVC decoder, implemented from ITU-T Rec. H.264 (ISO/IEC 14496-10).
//!
//! Supported: progressive (frame) coding, 4:2:0 and 4:2:2 at 8 to 10 bits, Baseline / Main / High / High 10 /
//! High 4:2:2 profiles — CAVLC and CABAC,
//! I/P/B slices, 8x8 transform, custom scaling matrices, weighted prediction, spatial/temporal direct,
//! multiple slices, long-term references and all MMCOs, deblocking.
//!
//! ```no_run
//! let mut dec = craft_video::h264::decode::Decoder::new();
//! let stream = std::fs::read("video.h264").unwrap();
//! for pic in dec.decode(&stream, 0).unwrap() {
//!     println!("{}x{} poc {}", pic.width, pic.height, pic.poc);
//! }
//! for pic in dec.flush() {
//!     let _ = pic;
//! }
//! ```

#![allow(clippy::needless_range_loop, clippy::type_complexity)]

mod cabac;
mod cabac_mb;
#[rustfmt::skip]
mod cabac_tables;
mod cavlc;
#[rustfmt::skip]
mod cavlc_tables;
mod deblock;
mod decoder;
pub mod dpb;
mod error;
mod inter;
mod intra;
mod mbtypes;
pub mod params;
mod picture;
pub use picture::set_plane_allocator;
#[cfg(test)]
mod hbd_tests;
pub mod slice;
mod slicedec;
#[cfg(test)]
mod synth_tests;
mod tables;
mod transform;

pub use decoder::{DecodeStats, Decoder, default_threads};
pub use error::{Error, Result};

/// Colour description from the VUI (ITU-T H.273 code points).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorInfo {
    /// `video_full_range_flag`.
    pub full_range: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}

/// A decoded, cropped planar picture: 4:2:0 or 4:2:2, 8 bits ([`Picture::y`] …) or deeper
/// ([`Picture::y16`] …). All three planes share one storage: [`Picture::is_deep`] tells which.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Cropped luma width.
    pub width: u32,
    /// Cropped luma height.
    pub height: u32,
    pub chroma_width: u32,
    pub chroma_height: u32,
    /// 8-bit samples (empty when [`Self::is_deep`]: either component is deeper than 8 bits).
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    /// All three planes of a picture with a component deeper than 8 bits ([`Self::is_deep`]; High
    /// 10, High 4:2:2), also when the other component has 8 bits: luma values below
    /// `1 << bit_depth`, chroma values below `1 << bit_depth_chroma`. Empty for 8-bit pictures.
    pub y16: Vec<u16>,
    pub u16: Vec<u16>,
    pub v16: Vec<u16>,
    /// Bits per luma sample (8 for most streams).
    pub bit_depth: u8,
    /// Bits per chroma sample (usually [`Self::bit_depth`]).
    pub bit_depth_chroma: u8,
    /// 4:2:2 chroma: the chroma planes have the luma height.
    pub chroma422: bool,
    /// Strides in samples.
    pub y_stride: usize,
    pub uv_stride: usize,
    /// Presentation timestamp passed to [`Decoder::decode`] with the access unit of this picture.
    pub pts: i64,
    /// Picture order count.
    pub poc: i32,
    /// IDR picture.
    pub key: bool,
    pub color: ColorInfo,
    /// Sample aspect ratio (0, 0 when unspecified).
    pub sar: (u16, u16),
    /// Decoded in draft mode ([`Decoder::set_draft`]): a non-reference picture whose deblocking
    /// filter was skipped. Its samples are approximate (not the conforming output); no other
    /// picture is affected.
    pub draft: bool,
}

impl Picture {
    /// The samples are in [`Self::y16`] / [`Self::u16`] / [`Self::v16`] (either component is deeper
    /// than 8 bits); otherwise in [`Self::y`] / [`Self::u`] / [`Self::v`].
    pub fn is_deep(&self) -> bool {
        self.bit_depth > 8 || self.bit_depth_chroma > 8
    }
}
