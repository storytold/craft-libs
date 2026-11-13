//! AV1, written from the *AV1 Bitstream & Decoding Process Specification* v1.0.0 with Errata 1.
//!
//! | Feature | Module | What |
//! |---|---|---|
//! | `av1-decode` | [`decode`] | Decoder |
//! | `av1-encode` | this module | Encoder ([`Encoder`]) |
//!
//! Feature `av1` turns on both.

#[cfg(feature = "av1-decode")]
pub mod decode;

#[cfg(feature = "av1-encode")]
mod encode;
#[cfg(feature = "av1-encode")]
pub use encode::*;
