//! Scan straightening: measure how far a raster's content is tilted, and rotate it level.
//!
//! **Angles** are degrees, positive counter-clockwise as displayed: a skew of `+1.0` means the
//! content's horizontal lines rise to the right by one degree, and [`rotate`] by `-1.0`
//! levels it.
//!
//! **Measuring** ([`estimate_skew`]) projects ink pixels perpendicular to each candidate
//! angle within ±[`MAX_SKEW_DEGREES`] and keeps the angle whose profile is most sharply
//! peaked: lines of text, table rules and title blocks all make that peak, with no OCR and no
//! language. A dark frame touching the image edge is ignored. A page too sparse or too dark to
//! project falls back to a vote by its edges. Both report a confidence; the caller picks the
//! floor.
//!
//! **Straightening** ([`straighten`]) measures, rotates and re-measures, and reports one of
//! three outcomes: straightened, already straight, or declined with the reason. "Can't tell"
//! is never reported as "straight", and a correction that leaves most of the tilt behind is
//! dropped rather than kept.
//!
//! Samples stay in their own layout ([`Layout`]: any of 1, 2, 4, 8 or 16 bits, any component
//! count, indexed or not). Encoding the result is the caller's: keep the source's codec family
//! (DCT stays DCT, a 1-bit image stays 1-bit) or a straightened scan grows several times.
//!
//! Nothing here panics on any input. Measuring is bounded by a grid of at most 1600 pixels a
//! side and at most 200,000 scored points; [`rotate`] and [`straighten`] also allocate one
//! output buffer the size of the input.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod estimate;
mod grid;
mod image;
mod rotate;
mod search;

pub use image::{Image, Layout, MAX_COMPONENTS, MAX_PIXELS, Paper};
pub use rotate::rotate;

/// The largest skew searched for, either way. Past it a page is more likely turned by 90°,
/// which is a page-rotation question, not a skew.
pub const MAX_SKEW_DEGREES: f64 = 15.0;

/// Why an image could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("unsupported layout: {0}")]
    Layout(&'static str),
    #[error("the image has {pixels} pixels; at most {max} are accepted")]
    TooLarge { pixels: u64, max: u64 },
    #[error("the samples are {got} bytes; the layout needs {needed}")]
    ShortBuffer { needed: usize, got: usize },
}

/// Which estimator measured a skew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Method {
    /// Projection of ink pixels.
    Projection,
    /// Vote of edge pixels weighted by their strength.
    Edges,
}

/// A measured skew.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Skew {
    /// Degrees within ±[`MAX_SKEW_DEGREES`], positive counter-clockwise as displayed.
    pub angle_degrees: f64,
    /// How sharply the best angle stands out, from 0 (no angle better than another) towards one.
    /// A page of text lines typically measures above 0.5; below about 0.2 the angle should not
    /// be trusted.
    pub confidence: f64,
    pub method: Method,
}

/// Choices for [`estimate_skew`] and [`straighten`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Options {
    /// A skew smaller than this, either way, is left alone.
    pub min_angle_degrees: f64,
    /// A measurement below this confidence is not acted on.
    pub min_confidence: f64,
    /// Try the edge vote when the projection cannot measure or is not confident.
    pub edge_fallback: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { min_angle_degrees: 0.1, min_confidence: 0.2, edge_fallback: true }
    }
}

impl Options {
    /// Set [`Options::min_angle_degrees`]; a value that is not finite is ignored.
    #[must_use]
    pub fn min_angle_degrees(self, degrees: f64) -> Self {
        if degrees.is_finite() { Self { min_angle_degrees: degrees.abs(), ..self } } else { self }
    }

    /// Set [`Options::min_confidence`]; a value that is not finite is ignored.
    #[must_use]
    pub fn min_confidence(self, confidence: f64) -> Self {
        if confidence.is_finite() { Self { min_confidence: confidence, ..self } } else { self }
    }

    #[must_use]
    pub fn edge_fallback(self, on: bool) -> Self {
        Self { edge_fallback: on, ..self }
    }
}

/// Why [`straighten`] declined to act on an image without calling it straight.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Reason {
    /// Neither estimator found anything to measure: blank, nearly blank, or no lines or edges.
    NothingToMeasure,
    /// The best measurement is below [`Options::min_confidence`].
    NotConfident(Skew),
    /// Rotating by the measurement did not remove most of the skew, so it was wrong.
    CorrectionFailed { skew: Skew, residual_degrees: Option<f64> },
}

/// What [`straighten`] did.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Outcome {
    /// Rotated level. `samples` has the input's layout; `residual_degrees` is the skew the
    /// rotated image still measures.
    Straightened { skew: Skew, residual_degrees: f64, samples: Vec<u8> },
    /// Measured with confidence, and within [`Options::min_angle_degrees`] of level.
    AlreadyStraight(Skew),
    /// Left alone, and not known to be straight.
    Declined(Reason),
}

/// Measure the skew of `image`. `None` when neither estimator finds anything to measure; a
/// result below `options.min_confidence` is still returned, for the caller to judge.
#[must_use]
pub fn estimate_skew(image: &Image<'_>, options: &Options) -> Option<Skew> {
    let background = image::background(image);
    let grid = grid::Grid::sample(image, &background);
    let skew = |(angle_degrees, confidence), method| Skew { angle_degrees, confidence, method };
    let projected = estimate::projection(&grid).map(|m| skew(m, Method::Projection));
    if !options.edge_fallback || projected.is_some_and(|p| p.confidence >= options.min_confidence) {
        return projected;
    }
    let voted = estimate::edges(&grid).map(|m| skew(m, Method::Edges));
    match (projected, voted) {
        (Some(p), Some(v)) if v.confidence > p.confidence => Some(v),
        (Some(p), _) => Some(p),
        (None, v) => v,
    }
}

/// Measure `image`, rotate it level when it needs it, and check the result by measuring again.
#[must_use]
pub fn straighten(image: &Image<'_>, options: &Options) -> Outcome {
    let Some(skew) = estimate_skew(image, options) else {
        return Outcome::Declined(Reason::NothingToMeasure);
    };
    if skew.confidence < options.min_confidence {
        return Outcome::Declined(Reason::NotConfident(skew));
    }
    if skew.angle_degrees.abs() < options.min_angle_degrees {
        return Outcome::AlreadyStraight(skew);
    }
    let samples = rotate(image, -skew.angle_degrees);
    let residual = Image::new(image.layout(), &samples).ok().and_then(|r| estimate_skew(&r, options)).map(|r| r.angle_degrees);
    match residual {
        Some(left) if corrected(skew.angle_degrees, left) => Outcome::Straightened { skew, residual_degrees: left, samples },
        residual_degrees => Outcome::Declined(Reason::CorrectionFailed { skew, residual_degrees }),
    }
}

/// Whether a rotation by `angle` that leaves `residual` behind was a real correction: it
/// removed at least 40% of the skew, or what is left is within one step of the search, which a
/// re-measure cannot resolve.
fn corrected(angle: f64, residual: f64) -> bool {
    residual.abs() <= (0.6 * angle.abs()).max(search::FINE_STEP)
}

#[cfg(test)]
mod tests {
    use super::corrected;

    #[test]
    fn a_residual_within_one_search_step_counts_as_corrected() {
        // 0.6 × 0.06 is under one step; without the step floor this small skew could never pass.
        assert!(corrected(0.06, 0.05));
        assert!(!corrected(2.0, 1.5));
    }
}
