//! Geometry for VectorCraft's built-in add-ons, one module per add-on, with the kernels they share.
//!
//! - [`keep_shape`]: remove anchors while refitting their neighbours' handles, so the outline keeps
//!   its shape within a tolerance (shared by every add-on that takes anchors away).
//! - [`cleanup`]: Artwork Cleanup's path repairs (collinear, duplicate and near anchors, tiny
//!   handles, nearly closed paths) and Rejoin Path Pieces.
//! - [`pathtools`]: Path Precision Tools' anchor edits, path readouts, extending open paths,
//!   finding where paths cross, sliding anchors along their path, merging touching edges and the
//!   Parametric Shape Tool's re-editable shapes.
//! - [`segment_reshape`]: bending a segment by dragging a point on it (shared by the tools that
//!   reshape segments directly).
//! - [`pen`]: the Smart Pen's straight and curved segments and its point edits.
//! - [`stencil`]: Stencil Bridges, the strips cut out of art so that a stencil cut from it keeps
//!   its islands attached.

pub mod cleanup;
pub mod keep_shape;
pub mod pathtools;
pub mod pen;
pub mod segment_reshape;
pub mod stencil;

pub use cleanup::{
    Rejoined, close_nearly_closed, merge_duplicate_anchors, near_point_groups, rejoin, remove_collinear_anchors, retract_tiny_handles,
};
pub use keep_shape::{PT_PER_TOLERANCE, SHARP_CORNER_DEG, remove_anchor_keep_shape, simplify_anchors_keep_shape, simplify_keep_shape};
pub use segment_reshape::{ReshapeMode, reshape_segment};
