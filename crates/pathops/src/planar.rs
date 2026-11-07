// Carved from VectorCraft's crates/pathops (src/planar.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! Planar maps: the faces and edges a set of paths divides the plane into.
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

use kurbo::{Affine, BezPath, CubicBez, ParamCurve, PathSeg, Point, Shape as _};
use linesweeper::topology::{Topology, WindingNumber};

use crate::FillRule;
use crate::boolean::{DEFAULT_PRECISION, Tidy, eps_for, is_sliver, snap_horizontals, tidy_segments};
use crate::path::{Seg, segs_to_bezpath};
use crate::pathfinder::{Region, Shape};

mod graph;
pub(crate) mod index;
mod live_paint;
mod map;
mod shape_builder;

#[cfg(test)]
use index::{Crossings, RectIndex, segment_footprint};
pub use live_paint::live_paint;
pub use map::interior_point;
pub(crate) use shape_builder::builder;
pub use shape_builder::{BuilderArrangement, SHAPE_BUILDER_MAX_SEGMENTS, has_open_path, shape_builder};

#[cfg(test)]
mod tests;
