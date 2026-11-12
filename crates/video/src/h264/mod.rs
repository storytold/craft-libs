//! ITU-T H.264 / AVC, written from ITU-T Rec. H.264 (ISO/IEC 14496-10).
//!
//! | Feature | Module | What |
//! |---|---|---|
//! | `h264-decode` | [`decode`] | Decoder: Baseline, Main and High, progressive 8-bit 4:2:0 |
//! | `h264-encode` | this module | Encoder: Constrained Baseline, Main and High ([`Encoder`]) |
//!
//! Feature `h264` turns on both.

#[cfg(feature = "h264-decode")]
pub mod decode;

#[cfg(feature = "h264-encode")]
mod encode;
#[cfg(feature = "h264-encode")]
pub use encode::*;
