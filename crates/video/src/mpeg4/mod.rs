//! MPEG-4 Part 2 video (ISO/IEC 14496-2, MPEG-4 Visual), the format Xvid and DivX write.
//!
//! | Feature | What |
//! |---|---|
//! | `mpeg4-decode` | Decoder: Simple and Advanced Simple profile ([`Decoder`]) |
//! | `mpeg4-encode` | Encoder: Simple profile, I- and P-VOPs ([`Encoder`]) |
//!
//! Feature `mpeg4` turns on both.

mod dct;
mod tables;

#[cfg(feature = "mpeg4-decode")]
mod decode;
#[cfg(feature = "mpeg4-decode")]
pub use decode::*;

#[cfg(feature = "mpeg4-encode")]
mod encode;
#[cfg(feature = "mpeg4-encode")]
pub use encode::*;
