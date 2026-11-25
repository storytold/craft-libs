//! Which faces of a planar map each edge separates, and every input each edge lies on.
//!
//! [`live_paint`](super::live_paint) gives each edge under its front-most input only, and says
//! nothing about the faces on its sides. [`adjacency`] reads both from the same planar map, so a
//! caller can tell which faces a line divides and which lines lie on top of each other.

use super::graph::{find, half_segs, twin, unshear};
use super::index::RectIndex;
use super::map::{Map, map_with};
use super::*;

/// A planar map of some paths with each edge's sides and inputs ([`adjacency`]).
#[derive(Clone, Debug, Default)]
pub struct Adjacency {
    pub faces: Vec<MapFace>,
    pub edges: Vec<MapEdge>,
    /// Bounded areas left out as slivers: narrower than the precision on average, such as a thin
    /// gap between two lines.
    pub slivers: Vec<PathData>,
    /// The outer boundary of each connected piece of the paths.
    pub outlines: Vec<MapOutline>,
}

/// A bounded face: its outline, then the outer boundaries of the pieces inside it (as holes), and
/// its area without them.
#[derive(Clone, Debug)]
pub struct MapFace {
    pub path: PathData,
    pub area: f64,
    /// The connected piece of the paths whose edges bound it.
    pub piece: usize,
}

/// An edge: a piece of one or more inputs between two places where paths meet.
#[derive(Clone, Debug)]
pub struct MapEdge {
    pub path: PathData,
    /// Every input it lies on (indices into the inputs, ascending).
    pub sources: Vec<usize>,
    /// The faces on each side of it as it runs along `path`, seen on the page (document
    /// coordinates, y down): `left` on the left. `None` outside every bounded face, or in a sliver.
    /// A piece of line that ends loose inside a face has that face on both sides.
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub piece: usize,
}

/// The outer boundary of a connected piece of the paths.
#[derive(Clone, Debug)]
pub struct MapOutline {
    pub path: PathData,
    pub piece: usize,
}

/// The planar map of `shapes`, every path an edge as in [`live_paint`](super::live_paint), with
/// faces refit to `precision` and slivers narrower than it left out. `None` when the paths can't be
/// swept (non-finite, too degenerate, or more than [`SHAPE_BUILDER_MAX_SEGMENTS`] segments), when
/// `precision` is not a positive number, and when no path has two anchors.
pub fn adjacency(shapes: &[Shape], precision: f64) -> Option<Adjacency> {
    if !(precision.is_finite() && precision > 0.0) {
        return None;
    }
    let segments: usize = shapes.iter().flat_map(|s| &s.path.subpaths).map(SubPath::segment_count).sum();
    if segments > SHAPE_BUILDER_MAX_SEGMENTS {
        return None;
    }
    let m = map_with(shapes, false, precision)?;
    Some(read(&m))
}

fn read(m: &Map) -> Adjacency {
    let path = |cycle: &[usize], closed: bool| {
        let segs: Vec<Seg> = cycle.iter().flat_map(|&h| half_segs(&m.edges, h)).collect();
        segs_to_subpath(&tidy_segments(segs, closed, &m.tidy), closed)
    };
    let unsheared = |subpaths: Vec<SubPath>| unshear(PathData::new(subpaths), &m.anchors, m.snap);

    // Connected pieces, by the vertices the edges join, numbered in the order they are first met.
    let vertices = m.edges.iter().flat_map(|e| e.ends).max().map_or(0, |v| v + 1);
    let mut parent: Vec<usize> = (0..vertices).collect();
    for e in &m.edges {
        let (a, b) = (find(&mut parent, e.ends[0]), find(&mut parent, e.ends[1]));
        if let Some(p) = parent.get_mut(a) {
            *p = b;
        }
    }
    let mut number: Vec<Option<usize>> = vec![None; vertices];
    let mut pieces = 0;
    let mut piece_of_vertex = |v: usize| {
        let root = find(&mut parent, v);
        match number.get_mut(root) {
            Some(Some(n)) => *n,
            Some(slot) => {
                *slot = Some(pieces);
                pieces += 1;
                pieces - 1
            }
            None => 0,
        }
    };
    let edge_pieces: Vec<usize> = m.edges.iter().map(|e| piece_of_vertex(e.ends[0])).collect();
    let piece_of = |cycle: &[usize]| cycle.first().and_then(|&h| edge_pieces.get(h / 2)).copied().unwrap_or(0);

    // The face on each half-edge's side: a face's own cycle, and the outer boundaries of the
    // pieces inside it, bound it.
    let mut face_of: Vec<Option<usize>> = vec![None; 2 * m.edges.len()];
    let mut faces = Vec::with_capacity(m.faces.len());
    for (i, (cycle, _, area)) in m.faces.iter().enumerate() {
        let holes: Vec<&(Vec<usize>, BezPath)> = m.face_holes.get(i).into_iter().flatten().filter_map(|&k| m.holes.get(k)).collect();
        for &h in cycle.iter().chain(holes.iter().flat_map(|(c, _)| c)) {
            if let Some(f) = face_of.get_mut(h) {
                *f = Some(i);
            }
        }
        let mut subpaths: Vec<SubPath> = path(cycle, true).into_iter().collect();
        subpaths.extend(holes.iter().filter_map(|(c, _)| path(c, true)));
        // A hole's outer boundary winds the other way: its area is negative.
        let area = area + holes.iter().map(|(_, bp)| bp.area()).sum::<f64>();
        faces.push(MapFace { path: unsheared(subpaths), area, piece: piece_of(cycle) });
    }

    // A face walk leaves out the edges that end loose (spurs): such an edge lies inside the face
    // around its middle, on both sides.
    let mut on_cycle = vec![false; 2 * m.edges.len()];
    let cycles = m.faces.iter().map(|f| &f.0).chain(m.holes.iter().map(|h| &h.0)).chain(m.slivers.iter().map(|s| &s.0));
    for &h in cycles.flatten() {
        if let Some(c) = on_cycle.get_mut(h) {
            *c = true;
        }
    }
    let spur = |e: usize| !on_cycle.get(2 * e).copied().unwrap_or(true) && !on_cycle.get(twin(2 * e)).copied().unwrap_or(true);
    // The faces by their bounds, so that each spur tests only the faces whose bounds hold it;
    // built only when there is a spur.
    let bezpaths: Vec<BezPath> = if (0..m.edges.len()).any(spur) { faces.iter().map(|f| f.path.to_bezpath()).collect() } else { vec![] };
    let face_index = RectIndex::new(bezpaths.iter().enumerate().map(|(i, bp)| (bp.bounding_box(), i)).collect());
    let face_around = |p: Point| {
        face_index
            .at(p, 0.0)
            .copied()
            .filter(|&i| bezpaths.get(i).is_some_and(|bp| bp.winding(p) != 0))
            .min_by(|&a, &b| faces.get(a).map_or(f64::INFINITY, |f| f.area).total_cmp(&faces.get(b).map_or(f64::INFINITY, |f| f.area)))
    };

    // A face cycle winds so its face is on the side where the cross product of the half-edge's
    // direction and the way to the face is positive: on the page, with y down, that is the right.
    let edges = m
        .edges
        .iter()
        .enumerate()
        .filter_map(|(e, edge)| {
            let sp = path(&[2 * e], false)?;
            let path = unsheared(vec![sp]);
            let (left, right) = if spur(e) {
                let around = path.to_bezpath().segments().next().map(|s| s.eval(0.5)).and_then(face_around);
                (around, around)
            } else {
                (face_of.get(twin(2 * e)).copied().flatten(), face_of.get(2 * e).copied().flatten())
            };
            Some(MapEdge { path, sources: edge.sources.clone(), left, right, piece: edge_pieces.get(e).copied().unwrap_or(0) })
        })
        .collect();
    let slivers = m.slivers.iter().filter_map(|(cycle, _)| path(cycle, true)).map(|sp| unsheared(vec![sp])).collect();
    let outlines =
        m.holes.iter().filter_map(|(cycle, _)| Some(MapOutline { path: unsheared(vec![path(cycle, true)?]), piece: piece_of(cycle) })).collect();
    Adjacency { faces, edges, slivers, outlines }
}

#[cfg(test)]
mod tests;
