//! H.263 video (ITU-T H.263), the format of 3GPP video, early Flash and videoconferencing.
//!
//! | Feature | What |
//! |---|---|
//! | `h263-decode` | Decoder: baseline, custom picture formats and Annexes D, F and J (`Decoder`, `Picture`) |
//! | `h263-encode` | Encoder: baseline and custom picture formats, I- and P-pictures (`Encoder`) |
//!
//! Both share the code tables, the transform and the interpolation of `crate::mpeg4`, so each
//! turns the matching `mpeg4-*` feature on.

#[cfg(feature = "h263-decode")]
mod decode;
#[cfg(feature = "h263-decode")]
pub use decode::*;
#[cfg(feature = "h263-encode")]
mod encode;
#[cfg(feature = "h263-encode")]
pub use encode::*;
