#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]
// Carved from VectorCraft's crates/pathops at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Path operations on [`kurbo::BezPath`], shared by the Craft apps: curve-preserving booleans, the
//! Pathfinder set (unite, minus front, intersect, exclude, divide, trim, merge, crop, outline, minus
//! back), planar regions, offset path, outline stroke, simplify and join.
//!
//! Booleans are curve-preserving: [`linesweeper`]'s robust sweep line works directly on cubic
//! Béziers, and the pieces it splits curves into are refitted afterwards so results carry few
//! anchors. A path is a `BezPath`; a filled path is read with a [`FillRule`], open subpaths
//! implicitly closed. Every operation takes hostile input as a matter of course: coordinates that
//! are not finite, or too large for the sweep's tolerance, and inputs too tangled to sweep are
//! refused with a [`PathOpsError`] (or, for the operations without a `Result`, give an empty or
//! unchanged path) rather than panicking or running without bound. The bounds are the defaults of
//! [`Limits`]; an app that knows its machine chooses its own with [`with_limits`].
//!
//! A panic inside the sweep (`linesweeper` panics on a few degenerate inputs) is caught with
//! [`std::panic::catch_unwind`] at the public functions and becomes [`PathOpsError::Degenerate`] or an
//! empty result. The panic hook still runs, nothing is caught in a build with `panic = "abort"`
//! (WebAssembly), and a panic in a closure the caller passes in is caught the same way.
//!
//! See the crate README for the API overview, the limits and their measured costs, and for where the
//! code came from.

mod boolean;
mod budget;
mod edit;
mod fit;
mod offset;
mod path;
mod pathfinder;
mod planar;
#[cfg(test)]
mod testutil;
mod work;

pub use boolean::{
    BoolOp, DEFAULT_PRECISION, MASK_WORK, area, boolean, boolean_n, normalize, try_area, try_boolean, try_boolean_n, try_normalize, try_unite_all,
    try_unite_all_with, unite_all, unite_all_with,
};
pub use budget::{Limits, MAX_ELEMENTS, MAX_SEGMENT_PAIRS, MAX_SWEEP_EVENT_WORK, MAX_SWEEP_LINE_PAIRS, with_limits};
pub use edit::{FIT_WORK, JOIN_WORK, MAX_FIT_POINTS, REDUNDANT_WORK, SimplifyOptions, join, remove_redundant_points, simplify, simplify_with};
pub use offset::{
    Cap, Join, MAX_OFFSET_SEGMENT_PAIRS, offset_path, outline_stroke, stroke_region, try_offset_path, try_outline_stroke, try_stroke_region,
};
pub use pathfinder::{PathfinderOp, Region, Shape, merge_regions, pathfinder, pathfinder_with, region_at, regions};
pub use planar::{BuilderArrangement, SHAPE_BUILDER_MAX_SEGMENTS, has_open_path, interior_point, live_paint, shape_builder};

/// How the inside of a path is decided. These are the only two rules there are, so the enum is
/// not `#[non_exhaustive]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FillRule {
    /// A point is inside when its winding number is not zero.
    #[default]
    NonZero,
    /// A point is inside when its winding number is odd.
    EvenOdd,
}

/// Errors from fallible operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PathOpsError {
    /// A coordinate or parameter is NaN or infinite.
    #[error("input contains NaN or infinite coordinates")]
    NonFinite,
    /// The sweep was given a path it could not close.
    #[error("input path could not be closed")]
    OpenPath,
    /// The shapes are too degenerate to sweep: coordinates so large that the sweep's tolerance is
    /// wider than a unit, or an input that makes the sweep fail.
    #[error("the shapes are too degenerate to combine")]
    Degenerate,
    /// The input is too big to combine in bounded work: it has more path elements than an
    /// operation reads ([`MAX_ELEMENTS`]), more pairs of segments with overlapping bounds than it
    /// allows ([`MAX_SEGMENT_PAIRS`], [`MAX_OFFSET_SEGMENT_PAIRS`]), more pairs of pieces at one
    /// height than a sweep takes ([`MAX_SWEEP_LINE_PAIRS`]), or a stroke outline of more elements
    /// than [`MAX_ELEMENTS`].
    #[error("the input has too many path elements, or its segments cross or overlap in too many places, to combine")]
    TooComplex,
}
