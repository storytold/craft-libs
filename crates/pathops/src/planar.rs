//! Live Paint planar maps: the faces and edges a set of paths divides the plane into.
//!
//! Unlike Pathfinder [`regions`](crate::regions), which splits the *filled areas* of closed shapes,
//! every path here is an edge, open ones included: three crossing lines enclose a triangle, a line
//! across a rectangle splits it in two, and an area enclosed by paths is a face whether or not
//! anything fills it.
//!
//! The sweep (`linesweeper`) only takes closed paths, so an open path goes in traced there and
//! back. Its two passes would cancel out as winding numbers, so the sweep counts boundary crossings
//! instead ([`Crossings`]), which never cancel. Faces are then walked on the planar graph of the
//! sweep's output segments; which inputs an edge came from and which fills cover a face are found
//! geometrically (the sweep's winding numbers can't be trusted once open paths and coincident
//! edges are in).
//!
//! The sweep runs in a sheared frame ([`SHEAR`]): it merges the two passes of a *horizontal* open
//! path as if they cancelled out, and in the sheared frame no edge drawn at a common angle (0°,
//! 45°, 90°…) is horizontal. The shear is by a power of two, so it maps typical coordinates
//! exactly and keeps exact coincidences (a corner on another shape's edge) exact.

use std::collections::HashMap;

use craft_geom::{FillRule, PathData, SubPath};
use kurbo::{Affine, BezPath, CubicBez, ParamCurve, ParamCurveArclen, PathSeg, Point, Shape as _};
use linesweeper::topology::{Topology, WindingNumber};

use crate::boolean::{DEFAULT_PRECISION, Seg, Tidy, eps_for, is_sliver, segs_to_subpath, snap_horizontals, tidy_segments, to_seg};
use crate::pathfinder::{Region, Shape};

mod adjacency;
mod graph;
mod index;
mod live_paint;
mod map;
mod shape_builder;

pub use adjacency::{Adjacency, MapEdge, MapFace, MapOutline, adjacency};
pub(crate) use graph::pin_ends;
#[cfg(test)]
use index::{Crossings, RectIndex, reference_budget, segment_footprint, work_done};
pub use live_paint::{find_gaps, live_paint, live_paint_with_gaps};
pub use map::interior_point;
pub(crate) use map::{Map, Rest, map};
pub use shape_builder::{
    BuilderArrangement, SHAPE_BUILDER_MAX_GAP, SHAPE_BUILDER_MAX_SEGMENTS, cut_out, encloses_area, has_open_path, run_around, shape_builder,
    shape_builder_with_gap,
};
pub(crate) use shape_builder::{builder, gap_inputs};

#[cfg(test)]
mod tests;
