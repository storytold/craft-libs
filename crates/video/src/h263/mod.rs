//! H.263 video (ITU-T H.263), the format of 3GPP video, early Flash and videoconferencing.
//!
//! | Feature | What |
//! |---|---|
//! | `h263-decode` | Decoder: baseline, custom picture formats and Annexes D, F and J ([`Decoder`], [`Picture`]) |
//!
//! The decoder shares the code tables, the inverse transform and the interpolation of
//! [`crate::mpeg4`], so `h263-decode` turns `mpeg4-decode` on.

#[cfg(feature = "h263-decode")]
mod decode;
#[cfg(feature = "h263-decode")]
pub use decode::*;
