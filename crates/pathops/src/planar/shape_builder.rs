// Carved from VectorCraft's crates/pathops (src/planar/shape_builder.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use super::map::build;
use super::*;
use crate::path::{has_filled_subpath, has_open_subpath, subs};

/// Most segments [`shape_builder`] sweeps into its planar map (open paths as edges, the gaps closed
/// paths enclose, outline edges); past this only the filled areas count. It is the default of
/// [`crate::Limits::shape_builder_segments`]. The map costs about what
/// the filled areas' arrangement does (both near linear in the input), so this only bounds the time
/// a call can take on art far past what a drawing holds. It counts segments, a closed subpath's
/// closing segment included; [`crate::MAX_ELEMENTS`] counts path elements (every move, line, curve
/// and close).
pub const SHAPE_BUILDER_MAX_SEGMENTS: usize = 65_536;

/// The arrangement [`shape_builder`] gives. Only this crate makes one (a field added later does
/// not break a caller that reads the ones it knows: `let BuilderArrangement { regions, .. } = ..`).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BuilderArrangement {
    pub regions: Vec<Region>,
    /// The pieces of the paths with no filled subpath (none closed, nor an open loop of one cubic that
    /// fills as a closed one), keyed by input index.
    pub lines: Vec<Shape>,
    /// The pieces of the other paths' outlines, keyed by input index (the front-most where inputs
    /// share one); only when asked for.
    pub edges: Vec<Shape>,
    /// The pieces of the paths with no filled subpath that lie under the outline of a closed path
    /// in front of them, or along another such path in front of them, keyed by input index. No
    /// piece of theirs is in `lines`; there the edge is the path's in front (in `edges` when it is
    /// a closed path's and they were asked for, else in `lines`).
    pub lines_under: Vec<Shape>,
}

/// Does any of `shapes` have an open subpath (a cutting edge for the planar map)?
pub fn has_open_path(shapes: &[Shape]) -> bool {
    shapes.iter().any(|s| has_open_subpath(&s.path))
}

/// The number of segments of `shapes`, closing segments included.
pub(crate) fn segment_count(shapes: &[Shape]) -> usize {
    shapes.iter().flat_map(|s| subs(&s.path)).map(|sp| sp.chain().len()).sum()
}

/// Regions of `shapes` (back → front), the pieces of their open paths and, with `edges`, the pieces
/// of their outlines (what erasing an edge deletes).
///
/// Closed paths bound regions with their filled areas, as in [`regions`](crate::regions), and the
/// gaps they enclose that nothing fills are regions too (empty `sources`), such as the middle of
/// four circles that touch. Open paths (the caller closes the ones whose fill should count) are
/// cutting edges: a line across a shape splits it, lines around an area enclose a region nothing
/// fills, and each path with no closed subpath comes back cut wherever another path meets it, as
/// pieces keyed by its index in `shapes`. An open subpath of one cubic that comes back to its start
/// round an area is the outline of that loop, and fills as a closed one does. Outlines are cut the same
/// way. With more than [`SHAPE_BUILDER_MAX_SEGMENTS`] segments or a map that can't be built, only the
/// filled areas of the closed subpaths (and of such loops) count (another open one fills nothing there
/// either) and there are no pieces.
pub fn shape_builder(shapes: &[Shape], edges: bool) -> BuilderArrangement {
    builder(shapes, edges, DEFAULT_PRECISION)
}

/// [`shape_builder`] refitting its results to `precision`.
pub(crate) fn builder(shapes: &[Shape], edges: bool, precision: f64) -> BuilderArrangement {
    if !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return BuilderArrangement::default();
    }
    let has_open = has_open_path(shapes);
    let planar = (segment_count(shapes) <= crate::budget::limits().shape_builder_segments)
        .then(|| {
            let keyed: Vec<Shape> = shapes.iter().enumerate().map(|(i, s)| Shape::new(s.path.clone(), s.rule, i as u64)).collect();
            build(&keyed, precision).unwrap_or_default()
        })
        .filter(|(faces, pieces, _)| !faces.is_empty() || !pieces.is_empty());
    let Some((faces, pieces, under)) = planar else {
        return BuilderArrangement { regions: filled_regions(shapes, precision), ..Default::default() };
    };
    // A shape with a filled subpath (an open loop of one cubic among them) gives outline edges, not lines.
    let lines: Vec<bool> = shapes.iter().map(|s| !has_filled_subpath(&s.path)).collect();
    let is_line = |i: u64| usize::try_from(i).ok().and_then(|i| lines.get(i)) == Some(&true);
    let (lines, outline): (Vec<Shape>, Vec<Shape>) = pieces.into_iter().partition(|e| is_line(e.key));
    let regions = if has_open {
        faces
    } else {
        // The filled areas from their arrangement, whose faces merge exactly, then the gaps.
        let mut regions = crate::pathfinder::regions_with(shapes, precision);
        regions.extend(faces.into_iter().filter(|f| f.sources.is_empty()));
        regions
    };
    BuilderArrangement {
        regions,
        lines: if has_open { lines } else { Vec::new() },
        edges: if edges { outline } else { Vec::new() },
        lines_under: if has_open { under } else { Vec::new() },
    }
}

/// The regions of the shapes' filled areas: those of their closed subpaths, and of open ones of one
/// cubic that come back to their start round an area, as in the map. Another open subpath is a cutting
/// edge, as in the map: it fills nothing, whatever it encloses. A shape left with no closed subpath is left out, so a
/// flood of cutting lines cannot push the arrangement past its size budget and take the filled shapes
/// down with it; the regions keep the original indices.
fn filled_regions(shapes: &[Shape], precision: f64) -> Vec<crate::Region> {
    if !shapes.iter().any(|s| has_open_subpath(&s.path)) {
        return crate::pathfinder::regions_with(shapes, precision);
    }
    let mut keep: Vec<usize> = Vec::new();
    let mut kept: Vec<Shape> = Vec::new();
    for (i, s) in shapes.iter().enumerate() {
        let mut closed = BezPath::new();
        for sp in subs(&s.path).into_iter().map(crate::path::Sub::closed_if_one_cubic_loop).filter(|sp| sp.closed && sp.draws_something()) {
            sp.push_closed(&mut closed);
        }
        if !closed.elements().is_empty() {
            keep.push(i);
            kept.push(Shape::new(closed, s.rule, s.key));
        }
    }
    crate::pathfinder::regions_with(&kept, precision)
        .into_iter()
        .map(|mut r| {
            r.sources = r.sources.iter().filter_map(|&s| keep.get(s).copied()).collect();
            r
        })
        .collect()
}
