//! Lottie (Bodymovin) JSON: the wire layer.
//!
//! A Lottie document is JSON, and most of its difficulty is in a few encodings that every layer
//! and shape reuses. This crate reads and writes those, on `serde_json` values, without a
//! document model of its own:
//!
//! - [`parse_document`] and [`Header`]: the document and its size, frame rate and frame range;
//! - [`read_prop`] / [`write_prop`]: an animatable property (`{a, k, x}`), its keyframes with the
//!   legacy end values, hold keys and spatial tangents, and [`Tangents`], the ease of a segment
//!   converted to and from speed and influence;
//! - [`Bezier`]: a path (`{c, v, i, o}`); [`Gradient`]: the flat stop list of a gradient;
//! - [`nums`], [`color`], [`hex_color`]: the tolerant value readers;
//! - [`base64`] and [`image_mime`]: embedded image assets as `data:` URIs.
//!
//! Implemented from the public Lottie format description (the lottie-spec 1.0 schema and the
//! LottieFiles format documentation); no player or exporter source was used.
//!
//! Which layer, shape, effect and text feature maps to what in an application's document is the
//! application's work. dotLottie (`.lottie`) is a ZIP archive around the same JSON and is not
//! read here.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod base64;
mod gradient;
mod path;
mod prop;
mod value;

pub use gradient::Gradient;
pub use path::Bezier;
pub use prop::{Ease, Key, KeyOut, Prop, PropValue, Tangents, is_animated, read_prop, tangent, write_keys, write_prop};
pub use serde_json::Value as Json;
pub use value::{color, hex_color, image_mime, nums, round6, to_hex};

/// Errors from reading a Lottie document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LottieError {
    #[error("not a Lottie document: {0}")]
    Parse(String),
}

/// Parses the JSON of a Lottie document. Nesting is bounded by `serde_json` (128 levels).
pub fn parse_document(bytes: &[u8]) -> Result<Json, LottieError> {
    serde_json::from_slice(bytes).map_err(|e| LottieError::Parse(e.to_string()))
}

/// The fields every document starts with.
#[derive(Debug, Clone, PartialEq)]
pub struct Header<'a> {
    /// `w`, `h`: the size in pixels.
    pub width: f64,
    pub height: f64,
    /// `fr`: frames per second; 30 when missing or not positive.
    pub frame_rate: f64,
    /// `ip`: the first frame; 0 when missing.
    pub in_point: f64,
    /// `op`: the frame after the last; one second after `ip` when missing, and at least one
    /// frame after it.
    pub out_point: f64,
    /// `nm`, when it is not empty.
    pub name: Option<&'a str>,
}

impl<'a> Header<'a> {
    /// Reads the header. A document without a numeric `w` and `h` is not a Lottie document.
    pub fn read(doc: &'a Json) -> Result<Self, LottieError> {
        let f = |k: &str| doc.get(k).and_then(Json::as_f64);
        let (Some(width), Some(height)) = (f("w"), f("h")) else { return Err(LottieError::Parse("missing `w`/`h`".into())) };
        let frame_rate = f("fr").filter(|v| *v > 0.0).unwrap_or(30.0);
        let in_point = f("ip").unwrap_or(0.0);
        let out_point = f("op").unwrap_or(in_point + frame_rate).max(in_point + 1.0);
        let name = doc.get("nm").and_then(Json::as_str).filter(|n| !n.is_empty());
        Ok(Self { width, height, frame_rate, in_point, out_point, name })
    }
}

#[cfg(test)]
mod tests;
