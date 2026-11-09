// Carved from VectorCraft's crates/pathops (src/planar/shape_builder.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.
// Brought up to storytold/vectorcraft 0945eaf2b3d4e93963aaa6d796b9efe53feaf624: its changes to this file (4cf4ea3, 500f74b) are covered here (see the README).

use super::map::try_build;
use super::*;
use crate::PathOpsError;
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
/// either) and there are no pieces: that is not the arrangement asked for, and [`try_shape_builder`]
/// says so instead.
pub fn shape_builder(shapes: &[Shape], edges: bool) -> BuilderArrangement {
    builder(shapes, edges, DEFAULT_PRECISION)
}

/// [`shape_builder`] that reports a planar map it cannot make, instead of falling back to the filled
/// areas: [`PathOpsError::TooComplex`] past the element limit, past [`SHAPE_BUILDER_MAX_SEGMENTS`]
/// segments ([`crate::Limits::shape_builder_segments`]), past the limit on pairs of segments or on the
/// work of winding points against fills and faces ([`crate::Limits::fill_work`]), and past the limit on
/// the faces of the filled areas ([`crate::Limits::mask_work`], [`crate::Limits::pass_work`]), which a map
/// that was made would be given without; [`PathOpsError::NonFinite`] for a point that is not finite, and
/// [`PathOpsError::Degenerate`] for one so large that the sweep cannot take it, in the shapes as they are
/// given (a closed loop of one cubic whose handle is no number is left out of the map, and is no excuse);
/// and [`PathOpsError::Degenerate`] when no frame of the sweep leaves the open paths unlevelled, or the
/// sweep fails. A map it makes is what [`shape_builder`] gives.
pub fn try_shape_builder(shapes: &[Shape], edges: bool) -> Result<BuilderArrangement, PathOpsError> {
    try_builder(shapes, edges, DEFAULT_PRECISION, true)
}

/// [`shape_builder`] refitting its results to `precision`.
pub(crate) fn builder(shapes: &[Shape], edges: bool, precision: f64) -> BuilderArrangement {
    try_builder(shapes, edges, precision, false).unwrap_or_default()
}

/// [`builder`]; with `strict`, a map that cannot be made is its error ([`try_shape_builder`]) where
/// it would fall back to the filled areas.
pub(crate) fn try_builder(shapes: &[Shape], edges: bool, precision: f64, strict: bool) -> Result<BuilderArrangement, PathOpsError> {
    if !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return if strict { Err(PathOpsError::TooComplex) } else { Ok(BuilderArrangement::default()) };
    }
    // Strictly, the coordinates are checked as given, before the map leaves out what draws nothing (a
    // closed loop of one cubic whose handle is no number is one): the error, not a map without it.
    if strict {
        crate::boolean::eps_for(shapes.iter().map(|s| &s.path))?;
    }
    let has_open = has_open_path(shapes);
    let mapped = if segment_count(shapes) <= crate::budget::limits().shape_builder_segments {
        let keyed: Vec<Shape> = shapes.iter().enumerate().map(|(i, s)| Shape::new(s.path.clone(), s.rule, i as u64)).collect();
        match try_build(&keyed, precision, strict) {
            Ok(map) => map,
            Err(e) if strict => return Err(e),
            Err(_) => None,
        }
    } else if strict {
        return Err(PathOpsError::TooComplex);
    } else {
        None
    };
    let planar = mapped.filter(|(faces, pieces, _)| !faces.is_empty() || !pieces.is_empty());
    let Some((faces, pieces, under)) = planar else {
        return Ok(BuilderArrangement { regions: filled_regions(shapes, precision, strict)?, ..Default::default() });
    };
    // A shape with a filled subpath (an open loop of one cubic among them) gives outline edges, not lines.
    let lines: Vec<bool> = shapes.iter().map(|s| !has_filled_subpath(&s.path)).collect();
    let is_line = |i: u64| usize::try_from(i).ok().and_then(|i| lines.get(i)) == Some(&true);
    let (lines, outline): (Vec<Shape>, Vec<Shape>) = pieces.into_iter().partition(|e| is_line(e.key));
    let regions = if has_open {
        faces
    } else {
        // The filled areas from their arrangement, whose faces merge exactly, then the gaps.
        let mut regions = filled(shapes, precision, strict)?;
        regions.extend(faces.into_iter().filter(|f| f.sources.is_empty()));
        regions
    };
    Ok(BuilderArrangement {
        regions,
        lines: if has_open { lines } else { Vec::new() },
        edges: if edges { outline } else { Vec::new() },
        lines_under: if has_open { under } else { Vec::new() },
    })
}

/// The faces of the filled areas of `shapes` ([`crate::pathfinder::try_regions_with`]); where they cannot be
/// found (past a limit, say), none, or with `strict` the error.
fn filled(shapes: &[Shape], precision: f64, strict: bool) -> Result<Vec<crate::Region>, PathOpsError> {
    match crate::pathfinder::try_regions_with(shapes, precision) {
        Err(e) if strict => Err(e),
        found => Ok(found.unwrap_or_default()),
    }
}

/// The regions of the shapes' filled areas: those of their closed subpaths, and of open ones of one
/// cubic that come back to their start round an area, as in the map. Another open subpath is a cutting
/// edge, as in the map: it fills nothing, whatever it encloses. A shape left with no closed subpath is left out, so a
/// flood of cutting lines cannot push the arrangement past its size budget and take the filled shapes
/// down with it; the regions keep the original indices.
fn filled_regions(shapes: &[Shape], precision: f64, strict: bool) -> Result<Vec<crate::Region>, PathOpsError> {
    if !shapes.iter().any(|s| has_open_subpath(&s.path)) {
        return filled(shapes, precision, strict);
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
    Ok(filled(&kept, precision, strict)?
        .into_iter()
        .map(|mut r| {
            r.sources = r.sources.iter().filter_map(|&s| keep.get(s).copied()).collect();
            r
        })
        .collect())
}
