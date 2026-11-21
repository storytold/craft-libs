// Carved from VectorCraft's crates/pathops (src/planar/live_paint.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use super::map::build;
use super::{DEFAULT_PRECISION, Region, Shape};

/// The planar map of `shapes` (back → front).
///
/// Faces are the bounded areas the paths enclose, holes included, each with the inputs whose fill
/// covers it (ascending; empty for an area only enclosed by paths). Edges are the paths split
/// wherever they meet another path, keyed by the input they came from (the front-most where
/// inputs share one). Empty when the paths can't be swept (non-finite or too degenerate), and when the map is
/// past a limit on the work to make it (pairs of segments, the winding of points against fills and faces,
/// [`crate::Limits::fill_work`], or the searches for a point inside each face, [`crate::Limits::search_work`]).
pub fn live_paint(shapes: &[Shape]) -> (Vec<Region>, Vec<Shape>) {
    build(shapes, DEFAULT_PRECISION).map(|(faces, edges, _)| (faces, edges)).unwrap_or_default()
}
