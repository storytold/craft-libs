//! MPEG-4 Part 2 video (ISO/IEC 14496-2, MPEG-4 Visual), the format Xvid and DivX write.
//!
//! | Feature | What |
//! |---|---|
//! | `mpeg4-decode` | Decoder: Simple and Advanced Simple profiles ([`Decoder`], [`Picture`]) |
//! | `mpeg4-encode` | Encoder: Simple profile, I- and P-VOPs ([`Encoder`]) |
//!
//! Feature `mpeg4` turns on both the decoder and the encoder.

/// Largest width or height (the VOL stores 13 bits; sizes are even).
pub const MAX_SIDE: u32 = 8190;

#[cfg(feature = "mpeg4-decode")]
mod decode;
#[cfg(feature = "mpeg4-decode")]
pub use decode::*;

#[cfg(feature = "mpeg4-encode")]
mod encode;
#[cfg(feature = "mpeg4-encode")]
pub use encode::*;
