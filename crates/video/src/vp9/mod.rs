//! VP9, written from the *VP9 Bitstream & Decoding Process Specification* v0.6.
//!
//! | Feature | Module | What |
//! |---|---|---|
//! | `vp9-decode` | [`decode`] | Decoder: profiles 0-3, all coding tools |
//! | `vp9-encode` | this module | Encoder: profile 0, 8-bit 4:2:0 ([`Vp9Encoder`]) |
//!
//! Feature `vp9` turns on both.

#[cfg(feature = "vp9-decode")]
pub mod decode;

#[cfg(feature = "vp9-encode")]
mod encode;
#[cfg(feature = "vp9-encode")]
pub use encode::*;
