//! Path operations for the Craft suite's 2D vector kernel (RFC-0030), carved from VectorCraft:
//! booleans (Pathfinder), shape builder regions, offset, outline stroke and simplify.
//!
//! Booleans are curve-preserving: `linesweeper`'s robust sweep-line works directly on cubic
//! Béziers, and the pieces it splits curves into are refitted afterwards so results carry few
//! anchors. See the crate README for the API overview.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod addons;
mod boolean;
mod budget;
mod edit;
mod fit;
mod offset;
mod pathfinder;
mod planar;
mod sweep_axis;
mod trap;

pub use boolean::{
    BoolOp, DEFAULT_PRECISION, area, boolean, boolean_n, normalize, try_area, try_boolean, try_boolean_n, try_normalize, try_unite_all,
    try_unite_all_with, unite_all, unite_all_with,
};
pub use edit::{
    AverageAxis, SIMPLIFY_LEVEL, SMOOTH_AUTO_LEVEL, SimplifiedPart, SimplifyOptions, add_anchor_points, auto_simplify_level, average, join,
    remove_anchor, remove_anchors, remove_redundant_points, simplify, simplify_level_tolerance, simplify_part, simplify_with, smooth, smooth_level,
    smooth_options, split_into_grid,
};
pub use fit::{fit_cubics, fit_cubics_through};
pub use offset::{Cap, Join, MAX_OFFSET_SEGMENT_PAIRS, offset_path, outline_stroke, stroke_region, try_offset_path};
pub use pathfinder::{FaceMerger, PathfinderOp, Region, Shape, merge_regions, pathfinder, pathfinder_with, region_at, regions, regions_within};
pub use planar::{
    Adjacency, BuilderArrangement, MapEdge, MapFace, MapOutline, SHAPE_BUILDER_MAX_GAP, SHAPE_BUILDER_MAX_SEGMENTS, adjacency, cut_out,
    encloses_area, find_gaps, has_open_path, interior_point, live_paint, live_paint_with_gaps, run_around, shape_builder, shape_builder_with_gap,
};
pub use trap::{TRAP_HEIGHT_WIDTH_RANGE, TRAP_MAX_WIDTH, TRAP_MIN_WIDTH, trap_band};

/// Errors from fallible operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathOpsError {
    #[error("input contains NaN or infinite coordinates")]
    NonFinite,
    #[error("input path could not be closed")]
    OpenPath,
    #[error("the shapes are too degenerate to combine")]
    Degenerate,
    #[error("the shapes cross themselves and each other too often to combine")]
    TooComplex,
}
