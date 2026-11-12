//! ITU-T H.265 / HEVC, written from ITU-T Rec. H.265 (ISO/IEC 23008-2).
//!
//! | Feature | Module | What |
//! |---|---|---|
//! | `hevc-decode` | [`decode`] | Decoder: Main and Main 10, all coding tools of those profiles |
//! | `hevc-encode` | this module | Encoder: Main and Main 10, 4:2:0 ([`Encoder`]) |
//!
//! Feature `hevc` turns on both.

#[cfg(feature = "hevc-decode")]
pub mod decode;

#[cfg(feature = "hevc-encode")]
mod encode;
#[cfg(feature = "hevc-encode")]
pub use encode::*;
