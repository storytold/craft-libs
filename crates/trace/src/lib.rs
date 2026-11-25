//! Image Trace for the Craft suite's 2D vector kernel (RFC-0030), carved from VectorCraft:
//! raster → vector.
//!
//! A clean-room tracer built from the published ideas behind bitmap tracers (the potrace paper
//! and classic colour quantisation), not from any existing implementation:
//!
//! 1. **Quantise** the image into a small palette: a luminance threshold (Black and White), 1-D
//!    k-means over the luminance histogram (Grayscale), or, in Color, over a 15-bit colour
//!    histogram as the [`Palette`] asks: median cut to a colour count followed by weighted k-means
//!    (Limited), median cut until every colour box is tight (Full Tone), one of those two by
//!    whether the image is flat art (Automatic), or the most used colours of a swatch library
//!    (Document Library). Transparent pixels are left out. The palette is made of the picture's own
//!    colours: the blends that anti-aliasing and blur leave along edges take none of it, colours that
//!    noise made of one colour are merged, and a blend pixel goes to one of the two colours it lies
//!    between (see `quantize`).
//!    A continuous-tone image (a photo) is first smoothed within its tones, edges kept (see
//!    `smooth`): otherwise its noise decides, pixel by pixel, which of two neighbouring tones a
//!    pixel gets, and every such pixel is a speck.
//! 2. **Remove noise**: whole 4-connected same-colour regions are merged, smallest first, into
//!    their most common neighbouring colour until no mergeable region is smaller than `noise`.
//!    Raising Noise continues the same merge sequence without splitting earlier regions.
//! 3. **Follow boundaries**: for each colour layer, the pixel-crack edges between inside and
//!    outside are linked into closed loops (inside on the right, right turns at saddles, so
//!    diagonal pixels stay separate, matching 4-connectivity). Outer loops run clockwise and
//!    holes counter-clockwise (y down), so the result fills correctly with either fill rule.
//! 4. **Sub-pixel edges**: each crack of a loop is moved to where the image says the edge really
//!    is (an anti-aliased or blurred edge pixel is part one tone, part the other), so the outline
//!    is right to a fraction of a pixel before any fitting; hard-edged images are read as
//!    staircases instead (see `subpixel`).
//!    Outlines are cut at their junctions (where three regions meet, and at the image's border)
//!    and each piece between two is fitted on its own, the same way from either side, so that two
//!    abutting regions have one and the same curve along the edge they share.
//! 5. **Curves**: the outline is fitted as Flat Logo mode fits its own (`logo::fit_outline`):
//!    corners the pixels rounded are put back, straight runs become lines, a round outline a
//!    true circle, and the rest as few cubic Béziers as the Paths tolerance allows; Snap Curves To
//!    Lines also straightens slightly curved runs and makes arcs of parts of circles.
//!
//! With **Create: Strokes**, the parts of a colour layer no wider than the stroke width are traced
//! as stroked centre lines instead (see `centerline`): placed in the middle of their ink to a
//! fraction of a pixel, as wide as their ink, and fitted as outlines are. Without Fills, wider
//! parts become stroked outlines.
//!
//! Colour layers are traced either *abutting* (each colour's own area; shapes share edges) or
//! *overlapping* (stacked: each layer also covers the layers above it within the areas that hold
//! its own color, so no hairline gaps). With Fills and without Create Strokes, an overlapping shape
//! is filled whole: the area of other colors that it encloses is traced on its own and drawn above
//! it, so each enclosed outline is fitted once. An area that holds a transparent pixel or a color
//! left out stays a hole.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod centerline;
mod contour;
mod deblock;
mod fit;
mod gradient;
mod group;
mod logo;
mod mosaic;
mod opaque;
mod quantize;
mod seam;
mod shapes;
mod smooth;
mod subpixel;

use std::sync::atomic::{AtomicUsize, Ordering};

use craft_geom::PathData;
use serde::{Deserialize, Serialize};

pub use contour::{Component, Loop, trace_mask};
pub use gradient::TracedGradient;
pub use mosaic::mosaic;
pub use opaque::{ALPHA_MAX_DECODE_BYTES, OUTLINE_MAX_ANCHORS, OUTLINE_MAX_PIXELS, decode_alpha, opaque_outline};
pub use quantize::{FULL_TONE_MAX, Quantized, TRANSPARENT, denoise, quantize};
pub use seam::{SeamFills, seam_fills};
pub use shapes::TracedShape;
use subpixel::Crack;

mod layers;
mod params;
mod pipeline;
mod raster;

use layers::parallel_map;
#[cfg(test)]
use layers::parallel_map_at_most;
#[cfg(test)]
pub(crate) use layers::shift_probe;
pub use params::{Method, Mode, PRESET_NAMES, Palette, TraceParams, enhanced_preset, preset, presets};
pub use pipeline::{TraceResult, TracedPath, trace, trace_watched, trace_within};
#[cfg(test)]
use pipeline::{clear_background, label_extents};
#[cfg(test)]
use raster::fit_size;
pub use raster::{MAX_PIXELS, MAX_SOURCE_PIXELS, Raster, TraceError};

#[cfg(test)]
mod tests;
