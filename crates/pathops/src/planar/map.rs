// Carved from VectorCraft's crates/pathops (src/planar/map.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use super::graph::{
    Edge, Fill, FillWork, Outlines, cycle_bezpath, far_end, find, half_segs, leaving_angle, origin, pin_ends, twin, unshear, without_spurs,
};
use super::index::{Crossings, Frame, RectIndex, mid, try_frame_for};
use super::*;
use crate::PathOpsError;
use crate::budget::SearchBudget;
use crate::work::{self, Work};

/// The planar graph of `shapes` (in the sweep's frame) with its faces walked: what [`build`] makes
/// the regions and edges of.
pub(crate) struct Map {
    edges: Vec<Edge>,
    around: Vec<Vec<usize>>,
    /// Bounded faces: their cycle of half-edges, path and area.
    faces: Vec<(Vec<usize>, BezPath, f64)>,
    /// The connected piece of the graph each face is in.
    face_piece: Vec<Option<usize>>,
    /// Outer boundaries of connected pieces, each with a point on it and its piece ([`place_holes`]
    /// finds the face each one is a hole in).
    holes: Vec<(Vec<usize>, BezPath)>,
    hole_at: Vec<(Option<Point>, Option<usize>)>,
    tidy: Tidy,
    /// The inputs' anchors (sorted by x) and how close results snap back onto them.
    anchors: Vec<Point>,
    snap: f64,
    /// The shear's inverse: from the sweep's frame back to the document.
    back: kurbo::Affine,
}

/// The planar map of `shapes`, whose faces and pieces are refit to `precision`, and whose slivers
/// thinner than it are dropped; and why there is none: `Ok(None)` when the shapes draw nothing, the frame's
/// error ([`try_frame_for`]), [`PathOpsError::TooComplex`] past the element limit or the limit on pairs
/// of segments, or [`PathOpsError::Degenerate`] when the sweep fails.
fn try_map_with(shapes: &[Shape], precision: f64) -> Result<Option<Map>, PathOpsError> {
    if !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return Err(PathOpsError::TooComplex);
    }
    // The sweep's frame: sheared so that no open path is level in it, and snapped to the sweep's tolerance.
    let Some(Frame { inputs, eps, back }) = try_frame_for(shapes)? else { return Ok(None) };
    // Counted on the snapped geometry, which is what the sweep sees: snapping can bring boxes into contact.
    if !crate::budget::sweep_fits(inputs.iter().map(|p| &p.0)) {
        return Err(PathOpsError::TooComplex);
    }
    // The map is held to a part of the sweep's budget: it takes several times what the sweep does.
    crate::meter::within_for_a_map(inputs.iter().map(|p| &p.0), eps)?;
    let tidy = Tidy::keeping(precision, inputs.iter().map(|p| &p.0));
    let (points, edges) = sweep_graph(&inputs, eps, precision).ok_or(PathOpsError::Degenerate)?;
    let nv = points.len();

    // Half-edges leaving each vertex, sorted by direction; walking a face, the next half-edge is
    // the one just clockwise of the way back, so bounded faces come out with positive area and
    // the outer boundary of each connected piece with negative area.
    let mut around: Vec<Vec<usize>> = vec![Vec::new(); nv];
    for h in 0..2 * edges.len() {
        if let Some(list) = origin(&edges, h).and_then(|v| around.get_mut(v)) {
            list.push(h);
        }
    }
    let mut slot_of = vec![0usize; 2 * edges.len()];
    let mut whole = super::graph::WHOLE_LINES_IN_ALL;
    for (v, list) in around.iter_mut().enumerate() {
        let Some(&at) = points.get(v) else { continue };
        // Half the shortest chord of the edges here: every one of them gets that far from it.
        let r = 0.5 * list.iter().filter_map(|&h| far_end(&edges, h).map(|p| p.distance(at))).fold(f64::INFINITY, f64::min);
        let mut keyed: Vec<(f64, usize)> = list.iter().map(|&h| (leaving_angle(&edges, h, at, r, &mut whole), h)).collect();
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
        *list = keyed.into_iter().map(|(_, h)| h).collect();
        for (k, &h) in list.iter().enumerate() {
            if let Some(s) = slot_of.get_mut(h) {
                *s = k;
            }
        }
    }
    let next = |h: usize| -> Option<usize> {
        let back = twin(h);
        let list = around.get(origin(&edges, back)?)?;
        let k = *slot_of.get(back)?;
        list.get((k + list.len() - 1) % list.len()).copied()
    };

    // Connected pieces of the graph.
    let mut parent: Vec<usize> = (0..nv).collect();
    for e in &edges {
        let (a, b) = (find(&mut parent, e.ends[0]), find(&mut parent, e.ends[1]));
        if let Some(p) = parent.get_mut(a) {
            *p = b;
        }
    }
    let piece: Vec<usize> = (0..nv).map(|v| find(&mut parent, v)).collect();
    let piece_of = |cycle: &[usize]| cycle.first().and_then(|&h| origin(&edges, h)).and_then(|v| piece.get(v)).copied();

    // Walk every cycle: positive ones are faces, negative ones outer boundaries (holes in the
    // face around them, if any).
    let mut seen = vec![false; 2 * edges.len()];
    let mut faces: Vec<(Vec<usize>, BezPath, f64)> = Vec::new();
    let mut holes: Vec<(Vec<usize>, BezPath)> = Vec::new();
    for start in 0..2 * edges.len() {
        if seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        let mut cycle = vec![];
        let mut h = start;
        while let Some(s) = seen.get_mut(h).filter(|s| !**s) {
            *s = true;
            cycle.push(h);
            match next(h) {
                Some(n) => h = n,
                None => break,
            }
        }
        let cycle = without_spurs(&cycle);
        if cycle.is_empty() {
            continue;
        }
        let bp = cycle_bezpath(&edges, &cycle);
        let area = crate::path::area_about_starts(&bp);
        // A positive cycle that is a sliver (thinner than the precision) is no face and is left out.
        if area > 0.0 && !is_sliver(&bp, tidy.precision) {
            faces.push((cycle, bp, area));
        } else if area < -(tidy.precision * tidy.precision) {
            holes.push((cycle, bp));
        }
    }

    // What placing the holes needs: a point on each, and the piece of each hole and face.
    let face_piece: Vec<Option<usize>> = faces.iter().map(|(cycle, _, _)| piece_of(cycle)).collect();
    let hole_at: Vec<(Option<Point>, Option<usize>)> = holes
        .iter()
        .map(|(cycle, _)| (cycle.first().and_then(|&h| origin(&edges, h)).and_then(|v| points.get(v)).copied(), piece_of(cycle)))
        .collect();

    // The inputs' anchors, which results land on again up to the sweep's tolerance.
    let snap = 2.0 * eps;
    let mut anchors: Vec<Point> = shapes.iter().flat_map(|s| s.path.elements().iter().filter_map(kurbo::PathEl::end_point)).collect();
    anchors.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    Ok(Some(Map { edges, around, faces, face_piece, holes, hole_at, tidy, anchors, snap, back }))
}

/// The holes of each face of `m`: each hole goes in the smallest face of another piece around it.
/// Each face whose bounds hold the hole's point is wound against it, its segments charged to `work`
/// first; `None` when that runs out (holes nested n deep cost about n² / 2 tests).
fn place_holes(m: &Map, work: &FillWork) -> Option<Vec<Vec<usize>>> {
    let face_index = RectIndex::new(m.faces.iter().enumerate().map(|(i, (_, bp, _))| (bp.bounding_box(), i)).collect());
    let mut face_holes: Vec<Vec<usize>> = vec![Vec::new(); m.faces.len()];
    for (k, &(at, own)) in m.hole_at.iter().enumerate() {
        let Some(p) = at else { continue };
        // The first of the smallest, as the faces are in order.
        let mut around_it: Option<(usize, f64)> = None;
        for &i in face_index.at(p, 0.0) {
            let (Some((_, bp, area)), Some(&piece)) = (m.faces.get(i), m.face_piece.get(i)) else { continue };
            if piece == own {
                continue;
            }
            if !work.charge(bp.elements().len()) {
                return None;
            }
            crate::work::add(Work::HoleWound, bp.elements().len());
            if crate::winding::winding(bp, p) != 0 && around_it.is_none_or(|(_, least)| area.total_cmp(&least).is_lt()) {
                around_it = Some((i, *area));
            }
        }
        if let Some(list) = around_it.and_then(|(i, _)| face_holes.get_mut(i)) {
            list.push(k);
        }
    }
    Some(face_holes)
}

/// The points where three or more edges of the planar map of `shapes` meet, by x and then y. Open
/// paths are edges of the map (traced there and back), so where one crosses another path is a
/// junction, which it is not in the arrangement of the filled areas, where the way there and the
/// way back cancel.
///
/// `Ok(None)` when the shapes draw nothing, and the map's error when it is not made ([`try_map_with`];
/// a panic of the sweep is [`PathOpsError::Degenerate`]).
pub(crate) fn try_junctions(shapes: &[Shape], precision: f64) -> Result<Option<Vec<Point>>, PathOpsError> {
    let Some(m) = guarded_map(shapes, precision)? else { return Ok(None) };
    let mut out: Vec<Point> = Vec::new();
    for list in m.around.iter().filter(|l| l.len() >= 3) {
        let Some(&h) = list.first() else { continue };
        let e = m.edges.get(h / 2).ok_or(PathOpsError::Degenerate)?;
        // Every edge at a vertex starts or ends exactly on it.
        let end = if h.is_multiple_of(2) { e.segs.first().map(|s| s.c.p0) } else { e.segs.last().map(|s| s.c.p3) };
        out.push(m.back * end.ok_or(PathOpsError::Degenerate)?);
    }
    out.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    Ok(Some(out))
}

/// [`try_map_with`], a panic of the sweep (on rare near-degenerate input) being
/// [`PathOpsError::Degenerate`].
fn guarded_map(shapes: &[Shape], precision: f64) -> Result<Option<Map>, PathOpsError> {
    crate::path::guarded(|| try_map_with(shapes, precision)).unwrap_or(Err(PathOpsError::Degenerate))
}

/// The planar graph of the sweep of `inputs` at the tolerance `eps`: its vertices and its edges. Points
/// within `eps` are one vertex, and so are the ends of copies of one edge and of stubs a rounding error
/// long (within a fraction of `precision` across). `None` when the sweep fails or panics.
fn sweep_graph(inputs: &[(BezPath, usize)], eps: f64, precision: f64) -> Option<(Vec<Point>, Vec<Edge>)> {
    // The sweep can panic on rare near-degenerate input; that must never take the app down.
    crate::work::add(crate::work::Work::Sweeps, 1);
    let sweep =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Topology::<Crossings>::from_paths(inputs.iter().map(|(p, i)| (p, *i)), eps)));
    let top = sweep.ok()?.ok()?;
    // The planar graph: vertices are the sweep's points, edges its segments. Points closer than
    // the sweep's tolerance are one vertex (it can put a corner that lies on another path's edge
    // a rounding error away from where it cuts that edge).
    let positions = top.compute_positions();
    let cell = 4.0 * eps;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut points: Vec<Point> = Vec::new();
    let mut vertex_id = |p: Point| {
        let (cx, cy) = ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
        let near = (cx - 1..=cx + 1)
            .flat_map(|x| (cy - 1..=cy + 1).map(move |y| (x, y)))
            .filter_map(|k| grid.get(&k))
            .flatten()
            .copied()
            .find(|&v| points.get(v).is_some_and(|q| q.distance(p) <= eps));
        near.unwrap_or_else(|| {
            points.push(p);
            grid.entry((cx, cy)).or_default().push(points.len() - 1);
            points.len() - 1
        })
    };
    let mut pieces: Vec<(Vec<Seg>, [usize; 2])> = Vec::new();
    for idx in top.segment_indices() {
        let segs: Vec<Seg> = positions[idx].path.segments().map(Seg::straight).collect();
        let ends = [vertex_id(top.point(idx.first_half()).to_kurbo()), vertex_id(top.point(idx.second_half()).to_kurbo())];
        if !segs.is_empty() && ends[0] != ends[1] {
            pieces.push((segs, ends));
        }
    }
    // The sweep can cut the paths through one crossing at points more than `eps` apart, so copies of
    // one edge (such as an open path's two passes) can end on different vertices. Pieces whose
    // middles and ends lie within `tol` of each other are such copies, and their ends are one vertex.
    let tol = 16.0 * eps;
    let cell_of = |p: Point| ((p.x / tol).floor() as i64, (p.y / tol).floor() as i64);
    let mids: Vec<Option<Point>> = pieces.iter().map(|(segs, _)| mid(segs)).collect();
    let mut by_mid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (k, m) in mids.iter().enumerate() {
        if let Some(m) = m {
            by_mid.entry(cell_of(*m)).or_default().push(k);
        }
    }
    let close = |a: usize, b: usize| matches!((points.get(a), points.get(b)), (Some(p), Some(q)) if p.distance(*q) <= tol);
    let mut joined: Vec<usize> = (0..points.len()).collect();
    // Where two passes of one path cross another (an open path's way there and back, coincident
    // outlines), the sweep can also join its two cuts, a rounding error apart, with a stub of that
    // length: a speck of a piece, and a vertex twice over. Such stubs are no edges: their ends are
    // one vertex. The sweep moves points by a few `eps` at most up and down its frame; along a path
    // that runs nearly level in it (where the height a cut is found at pins its position down
    // least) a cut can slide further, here up to a small fraction of the precision results are
    // tidied to, below which no edge is drawn anyway. A vertex's cluster stays within those bounds
    // as a whole: a run of short, nearly level segments (a finely flattened or traced path) is real
    // geometry, not one stub after another, and must not collapse into one vertex.
    let (stub_y, stub_x) = (4.0 * eps, (DEFAULT_PRECISION / 16.0).max(4.0 * eps));
    let mut extent: Vec<kurbo::Rect> = points.iter().map(|&p| kurbo::Rect::from_points(p, p)).collect();
    for (segs, [v0, v1]) in &pieces {
        let Some(&p0) = points.get(*v0) else { continue };
        if !segs.iter().flat_map(|s| [s.c.p1, s.c.p2, s.c.p3]).all(|p| (p.y - p0.y).abs() <= stub_y && (p.x - p0.x).abs() <= stub_x) {
            continue;
        }
        let (a, b) = (find(&mut joined, *v0), find(&mut joined, *v1));
        let (Some(&ra), Some(&rb)) = (extent.get(a), extent.get(b)) else { continue };
        let both = ra.union(rb);
        if a == b || both.width() > stub_x || both.height() > stub_y {
            continue;
        }
        let (root, child) = (a.min(b), a.max(b));
        if let Some(p) = joined.get_mut(child) {
            *p = root;
        }
        if let Some(r) = extent.get_mut(root) {
            *r = both;
        }
    }
    for (k, (_, [a, b])) in pieces.iter().enumerate() {
        let Some(&Some(m)) = mids.get(k) else { continue };
        let (cx, cy) = cell_of(m);
        let others = (cx - 1..=cx + 1).flat_map(|x| (cy - 1..=cy + 1).map(move |y| (x, y))).filter_map(|c| by_mid.get(&c)).flatten();
        for &j in others.filter(|&&j| j > k) {
            let (Some((_, [c, d])), Some(&Some(n))) = (pieces.get(j), mids.get(j)) else { continue };
            if m.distance(n) > tol {
                continue;
            }
            let pairs = if close(*a, *c) && close(*b, *d) {
                [(*a, *c), (*b, *d)]
            } else if close(*a, *d) && close(*b, *c) {
                [(*a, *d), (*b, *c)]
            } else {
                continue;
            };
            for (x, y) in pairs {
                let (x, y) = (find(&mut joined, x), find(&mut joined, y));
                if let Some(r) = joined.get_mut(x.max(y)) {
                    *r = x.min(y);
                }
            }
        }
    }
    let mut edges: Vec<Edge> = Vec::new();
    let mut between: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    let outlines = Outlines::new(inputs, eps);
    for (mut segs, ends) in pieces {
        let ends = ends.map(|v| find(&mut joined, v));
        let (Some(&p0), Some(&p1)) = (points.get(ends[0]), points.get(ends[1])) else { continue };
        if ends[0] == ends[1] {
            continue;
        }
        // Source attribution belongs to the sweep piece: pinning its endpoints can move the
        // middle off the original outlines even though it still separates the same faces.
        let mut sources = mid(&segs).map(|m| outlines.sources_at(m)).unwrap_or_default();
        // Every edge at a vertex starts or ends exactly on it, so faces meeting there share its
        // coordinates: merging them (a union) then cancels their common edge, which it can't when
        // each face carries its own rounding of the corner (VectorCraft issue 893).
        pin_ends(&mut segs, p0, p1);
        if sources.is_empty() {
            sources = mid(&segs).map(|m| outlines.sources_at(m)).unwrap_or_default();
        }
        let Some(&source) = sources.last() else { continue };
        // Coincident pieces the sweep left apart are one edge (the front-most input's), lying on
        // the inputs of both.
        let key = (ends[0].min(ends[1]), ends[0].max(ends[1]));
        let here = mid(&segs);
        let twin_of = between.get(&key).and_then(|v| {
            v.iter().copied().find(|&e| {
                let other = edges.get(e).and_then(|e| mid(&e.segs));
                matches!((here, other), (Some(a), Some(b)) if a.distance(b) < precision)
            })
        });
        if let Some(e) = twin_of {
            if let Some(e) = edges.get_mut(e) {
                e.source = e.source.max(source);
                e.sources.extend(sources);
                e.sources.sort_unstable();
                e.sources.dedup();
            }
            continue;
        }
        between.entry(key).or_default().push(edges.len());
        edges.push(Edge { segs, ends, source, sources });
    }
    Some((points, edges))
}

/// What [`build`] gives: the faces, the edges, and the pieces of the open paths under other paths.
pub(super) type Built = (Vec<Region>, Vec<Shape>, Vec<Shape>);

/// Faces and edges of `shapes`, and the pieces of the open paths (those with no filled subpath, [`crate::path::has_filled_subpath`])
/// that lie under another path in front of them, whose edges are that path's: under a closed
/// path's outline, or along an open path in front.
pub(super) fn build(shapes: &[Shape], precision: f64) -> Option<Built> {
    try_build(shapes, precision, false).ok().flatten()
}

#[cfg(test)]
thread_local! {
    /// Makes [`try_build`] find no point in a face of less area than this (0: none), which no face thicker than
    /// the precision comes to, so that a test can see what it does when it finds none.
    pub(super) static NO_POINTS_BELOW: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

/// The faces, edges and pieces under other paths of [`build`], and why there are none: `Ok(None)` when
/// the shapes draw nothing, the map's error ([`try_map_with`]; a panic of the sweep is
/// [`PathOpsError::Degenerate`]), and [`PathOpsError::TooComplex`] past the limit on the work of winding
/// points against fills and faces ([`crate::Limits::fill_work`]) or on the searches for a point inside each
/// face ([`crate::Limits::search_work`]); and, with `strict`, [`PathOpsError::Degenerate`]
/// for a face thicker than the precision that no point is found in (what covers it is not known, and it would
/// be listed as a gap that nothing fills).
pub(super) fn try_build(shapes: &[Shape], precision: f64, strict: bool) -> Result<Option<Built>, PathOpsError> {
    let Some(m) = guarded_map(shapes, precision)? else { return Ok(None) };
    // The winding of points against faces and fills, charged before each test: nested contours cost
    // the square of their depth, and past the limit there is no map.
    let work = FillWork::new();
    let face_holes = place_holes(&m, &work).ok_or(PathOpsError::TooComplex)?;
    let Map { edges, around, faces, holes, tidy, anchors, snap, back, .. } = &m;
    let snap = *snap;
    let fills: Vec<Fill> = shapes.iter().map(Fill::new).collect();
    // The fills whose bounds hold a point, in the order of the shapes: not every fill is tested for every face.
    let fill_index = RectIndex::new(fills.iter().enumerate().map(|(i, f)| (f.bounds, i)).collect());
    let subpath = |cycle: &[usize]| {
        let segs: Vec<Seg> = cycle.iter().flat_map(|&h| half_segs(edges, h)).collect();
        segs_to_bezpath(&tidy_segments(segs, true, tidy), true)
    };
    // What the searches for a point inside a face may spend, in all the faces of the map ([`crate::Limits::search_work`]).
    let search = SearchBudget::new();
    let mut regions = Vec::with_capacity(faces.len());
    for ((cycle, _, _), hs) in faces.iter().zip(&face_holes) {
        let Some(mut path) = subpath(cycle) else { continue };
        for sp in hs.iter().filter_map(|&k| holes.get(k)).filter_map(|(c, _)| subpath(c)) {
            path.extend(sp.iter());
        }
        let path = unshear(path, anchors, snap, *back);
        // A face thinner than the flattening the point is looked for in (a thousandth of its size: a lens a
        // hundred long and 0.03 wide between two curves has none of its middles in it) needs a finer outline: the
        // outline's flattening is made a quarter of the precision, or an eighth of what it was where that is less.
        // Both searches are charged to the budget of the searches after a sweep ([`crate::Limits::search_work`]), a step an edge, before the edges
        // they look at are flattened (the lines a curve is flattened to, at the most, [`crate::path::flatten_charged`]) and before each height they
        // are crossed with a line at: a face that is thin makes far more edges at the finer outline, and a face is searched at up to 150 heights.
        // An edge at a height takes a nanosecond and a line flattened 4 (measured, in a release build, on the crescents of circles 100,000 across),
        // a step 4.6 (the budget is 5 s): so the whole of it spent on this is 1 to 4 s, and out of it the map is not built, as for the windings.
        // A coarser outline would lose the sources of the face, so there is none.
        let charge = |edges: usize| search.charge(edges);
        let inside = (|| -> Result<Option<Point>, Exhausted> {
            let Some(first) = flattening_of(&path) else { return Ok(None) };
            match interior_point_charged(&path, first, &charge)? {
                Some(p) => Ok(Some(p)),
                None => interior_point_charged(&path, retry_tolerance(precision, first), &charge),
            }
        })()
        .map_err(|Exhausted| PathOpsError::TooComplex)?;
        #[cfg(test)]
        let inside = if path.area().abs() < NO_POINTS_BELOW.with(std::cell::Cell::get) { None } else { inside };
        if inside.is_none() && strict && !crate::boolean::is_sliver(&path, precision) {
            return Err(PathOpsError::Degenerate);
        }
        let mut sources = Vec::new();
        if let Some(p) = inside {
            for &i in fill_index.at(p, 0.0) {
                if fills.get(i).ok_or(PathOpsError::Degenerate)?.contains(p, &work).ok_or(PathOpsError::TooComplex)? {
                    sources.push(i);
                }
            }
        }
        regions.push(Region { path, sources });
    }

    // Edges: chains of segments through vertices where nothing else meets.
    let to_shapes = |chains: Vec<(Vec<Seg>, bool, usize)>| -> Vec<Shape> {
        chains
            .into_iter()
            .filter_map(|(segs, closed, source)| {
                let sp = segs_to_bezpath(&tidy_segments(segs, closed, tidy), closed)?;
                Some(Shape::new(unshear(sp, anchors, snap, *back), FillRule::NonZero, shapes.get(source)?.key))
            })
            .collect()
    };
    let lines: Vec<bool> = shapes.iter().map(|s| !crate::path::has_filled_subpath(&s.path)).collect();
    let is_line = |i: usize| lines.get(i) == Some(&true);
    // An edge is the front-most input's. Every other open path along it has a piece there too.
    let edge_shapes = to_shapes(chains_by(edges, around, |e| vec![e.source]));
    let under = to_shapes(chains_by(edges, around, |e| e.sources.iter().rev().copied().filter(|&i| i != e.source && is_line(i)).collect()));
    Ok(Some((regions, edge_shapes, under)))
}

/// The edges that `keys` gives each input, joined into chains through the vertices where only two
/// edges of the same input meet, each with that input. An edge that several inputs share is in a
/// chain for each of them. A chain is closed when it comes back round to where it started.
fn chains_by(edges: &[Edge], around: &[Vec<usize>], keys: impl Fn(&Edge) -> Vec<usize>) -> Vec<(Vec<Seg>, bool, usize)> {
    let keys: Vec<Vec<usize>> = edges.iter().map(&keys).collect();
    // Where input `k` is among the inputs half-edge `h` gives.
    let slot = |h: usize, k: usize| keys.get(h / 2).and_then(|ks| ks.iter().position(|&x| x == k));
    let passes = |v: usize, k: usize| -> Option<(usize, usize)> {
        let list = around.get(v)?;
        let (&a, &b) = (list.first()?, list.get(1)?);
        (list.len() == 2 && slot(a, k).is_some() && slot(b, k).is_some()).then_some((a, b))
    };
    // Each edge's inputs, and whether its chain for each has been walked.
    let mut used: Vec<Vec<bool>> = keys.iter().map(|ks| vec![false; ks.len()]).collect();
    let mut chains: Vec<(Vec<Seg>, bool, usize)> = Vec::new();
    // A chain of input `k` from half-edge `h0` up to the next vertex where it doesn't pass straight through.
    let walk = |h0: usize, k: usize, used: &mut Vec<Vec<bool>>| -> Option<(Vec<Seg>, bool, usize)> {
        let mut segs = Vec::new();
        let mut h = h0;
        loop {
            let u = used.get_mut(h / 2)?.get_mut(slot(h, k)?)?;
            if *u {
                return Some((segs, true, k));
            }
            *u = true;
            segs.extend(half_segs(edges, h));
            let v = origin(edges, twin(h))?;
            let Some((a, b)) = passes(v, k) else { return Some((segs, origin(edges, h0) == Some(v), k)) };
            h = if a == twin(h) { b } else { a };
        }
    };
    let unwalked = |used: &[Vec<bool>], h: usize, k: usize| slot(h, k).is_some_and(|s| used.get(h / 2).and_then(|u| u.get(s)) == Some(&false));
    for v in 0..around.len() {
        for &h in around.get(v).map(Vec::as_slice).unwrap_or_default() {
            for &k in keys.get(h / 2).map(Vec::as_slice).unwrap_or_default() {
                if passes(v, k).is_none()
                    && unwalked(&used, h, k)
                    && let Some(c) = walk(h, k, &mut used)
                {
                    chains.push(c);
                }
            }
        }
    }
    // What's left are closed loops with no junction on them.
    for e in 0..edges.len() {
        for &k in keys.get(e).map(Vec::as_slice).unwrap_or_default() {
            if unwalked(&used, 2 * e, k)
                && let Some(c) = walk(2 * e, k, &mut used)
            {
                chains.push(c);
            }
        }
    }
    chains
}

/// A point strictly inside a filled path, well away from its edges where it can be. None for a path
/// with no area, or one whose points are not finite, or one of more elements than [`crate::MAX_ELEMENTS`].
pub fn interior_point(path: &BezPath) -> Option<Point> {
    // The size first, before the path is copied: a path of more elements than the limit that begins with a close would be copied whole.
    if !crate::budget::fits_elements([path]) {
        return None;
    }
    // A close at the start closes nothing, and kurbo's bounding box panics on it.
    let path = crate::path::without_leading_closes(path);
    interior_point_within(&path, flattening_of(&path)?)
}

/// The tolerance a face that the first outline (flattened to `first`, a thousandth of its size) finds no point in is flattened to: a quarter of
/// the precision, or an eighth of what the first was where that is less.
pub(super) fn retry_tolerance(precision: f64, first: f64) -> f64 {
    (precision / 4.0).min(first / 8.0)
}

/// The search for a point inside a face spent more than it was allowed to ([`crate::Limits::search_work`]).
pub(super) struct Exhausted;

/// The tolerance [`interior_point`] flattens `path` to: a thousandth of its size, and not under 1e-4. `None`
/// for a path whose points are not finite or that draws nothing.
fn flattening_of(path: &BezPath) -> Option<f64> {
    let bb = finite_bounds(path)?;
    Some((bb.width().max(bb.height()) * 1e-3).max(1e-4))
}

/// The bounds of `path` when its points are finite and its size is, else None.
fn finite_bounds(path: &BezPath) -> Option<kurbo::Rect> {
    if path.elements().is_empty() || !crate::budget::fits_elements([path]) || !crate::path::is_finite(path) {
        return None;
    }
    let bb = path.bounding_box();
    (bb.width().is_finite() && bb.height().is_finite()).then_some(bb)
}

/// [`interior_point`] with the path's curves flattened to within `tolerance` (taken coarser where
/// a curve would need more than a few thousand lines, [`crate::path::flatten`]).
fn interior_point_within(path: &BezPath, tolerance: f64) -> Option<Point> {
    interior_point_charged(path, tolerance, &|_| true).ok().flatten()
}

/// [`interior_point_within`] that asks `charge` for the edges it is about to flatten (each curve's, before it is) and for the edges it is about to
/// cross with a line (before each height), and gives up with [`Exhausted`] when `charge` says there is no more.
pub(super) fn interior_point_charged(path: &BezPath, tolerance: f64, charge: &dyn Fn(usize) -> bool) -> Result<Option<Point>, Exhausted> {
    let Some(bb) = finite_bounds(path) else { return Ok(None) };
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Ok(None);
    }
    let mut edges: Vec<(Point, Point)> = vec![];
    let (mut first, mut last) = (Point::ZERO, Point::ZERO);
    let flattened = crate::path::flatten_charged(path.iter(), tolerance, charge, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if last != first {
                edges.push((last, first));
            }
            first = p;
            last = p;
        }
        kurbo::PathEl::LineTo(p) => {
            edges.push((last, p));
            last = p;
        }
        kurbo::PathEl::ClosePath => {
            if last != first {
                edges.push((last, first));
            }
            last = first;
        }
        _ => {}
    });
    if !flattened {
        return Err(Exhausted);
    }
    if last != first {
        edges.push((last, first));
    }
    if let Some(found) = inside_edges(path, &edges, bb, charge)? {
        return Ok(Some(found));
    }
    // Nothing found. Edges that run back over each other cancel in the winding and bound nothing: a path
    // that retraces itself (out and back to a stub, as an open path in a planar map is traced) puts
    // vertices at heights where there is no crossing of use, and they use up the few heights tried,
    // so the search is made again on what does not cancel, in its own bounds.
    let before = edges.len();
    let kept = without_retraces(edges);
    if kept.len() == before {
        return Ok(None);
    }
    let Some(bounds) = bounds_of_edges(&kept) else { return Ok(None) };
    inside_edges(path, &kept, bounds, charge)
}

/// A point inside `path` found from its flattened `edges`, whose box is `bb`: the middle of a gap
/// of the polygon between two crossings, at a few heights, where the path winds. `charge` is asked for the edges, before each height they are
/// crossed with a line at ([`Exhausted`] when it says no).
fn inside_edges(bp: &BezPath, edges: &[(Point, Point)], bb: kurbo::Rect, charge: &dyn Fn(usize) -> bool) -> Result<Option<Point>, Exhausted> {
    // The gaps between consecutive crossings of the polygon with a line at each of a few heights,
    // each with the width the polygon has at its middle: the widest where the polygon winds. A gap
    // must be wider than the rounding of its two crossings: one thinner is where edges retrace each
    // other a hair apart (out and back along a slanted line, through points rounded each way), not a part.
    let gaps_at = |y: f64, gaps: &mut Vec<(f64, Point)>| -> bool {
        if !charge(edges.len()) {
            return false;
        }
        work::add(Work::InteriorHeights, 1);
        // Each crossing's x and which way the edge goes, so that the winding number in a gap is
        // the sum of the crossings before it (one sort and one pass, not a winding per gap), and how
        // far rounding can have put it ([`crossing_rounding`]): by the spacing of the numbers there, and not
        // by a multiple of it that adds up to more than a part is wide. That is of the x itself, not of the
        // height of the path or of its widest x: two unit squares ten billion apart in height, or four
        // hundred trillion out in x, are parts. A vertical edge crosses at its x exactly (`t` times nothing
        // is nothing), with no rounding to allow for. A slanted one at 1e15, where the spacing is 0.125,
        // allowed 0.9 for each crossing (eight times the spacing), so a gap under 1.8 was a retrace: a
        // parallelogram a unit wide had no point, though it holds seven numbers.
        let mut xs: Vec<(f64, i32, f64)> = edges
            .iter()
            .filter(|(a, c)| (a.y <= y) != (c.y <= y))
            .map(|(a, c)| {
                let x = a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x);
                let off = if c.x == a.x { 0.0 } else { crossing_rounding(*a, *c, x) };
                (x, if c.y > a.y { 1 } else { -1 }, off)
            })
            .collect();
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        // What of the polygon lies on the line itself: its level edges at this height, and its vertices
        // there, each as a stretch of x. A height that rounds onto an edge (2 apart at 1e16) is one.
        let mut on_line: Vec<(f64, f64)> = edges
            .iter()
            .flat_map(|(a, c)| [(*a, *c), (*a, *a)])
            .filter(|(a, c)| a.y == y && c.y == y)
            .map(|(a, c)| (a.x.min(c.x), a.x.max(c.x)))
            .collect();
        on_line.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The stretches starting at or before `x`, and whether one of them reaches it (a stretch can
        // cover the next ones, so the furthest reach of those before it is what counts).
        let mut reach: Vec<f64> = Vec::with_capacity(on_line.len());
        for &(_, hi) in &on_line {
            reach.push(reach.last().map_or(hi, |&r: &f64| r.max(hi)));
        }
        let on_edge = |x: f64| {
            let k = on_line.partition_point(|&(lo, _)| lo <= x);
            k > 0 && reach.get(k - 1).is_some_and(|&r| r >= x)
        };
        let mut winding = 0;
        for w in xs.windows(2) {
            winding += w[0].1;
            let width = w[1].0 - w[0].0;
            // The middle, where it is strictly between the two: between adjacent numbers it rounds onto
            // one of them, an edge, which the winding can count as inside. Nor on an edge or a vertex
            // along the line.
            let middle = (w[0].0 + w[1].0) / 2.0;
            if winding != 0 && width > w[0].2 + w[1].2 && middle > w[0].0 && middle < w[1].0 && !on_edge(middle) {
                gaps.push((width, Point::new(middle, y)));
            }
        }
        true
    };
    // The widest first, the earlier of equal ones first. The polygon is the path flattened, so
    // the winding of the path itself decides, at the few widest middles (a middle the polygon has
    // inside the path has outside is a gap too thin to be one).
    let widest_inside = |mut gaps: Vec<(f64, Point)>| -> Option<Point> {
        gaps.sort_by(|a, b| b.0.total_cmp(&a.0));
        gaps.into_iter()
            .take(MAX_WINDING_TESTS)
            .find(|(_, m)| {
                work::add(Work::Winding, 1);
                crate::winding::winding(bp, *m) != 0
            })
            .map(|(_, m)| m)
    };
    let mut gaps: Vec<(f64, Point)> = vec![];
    for f in [0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875, 0.0625, 0.9375] {
        // A little off the round fractions, which on art drawn to a grid fall on the height of
        // some corner, where testing the point against other paths is unreliable.
        if !gaps_at(bb.y0 + bb.height() * (f + 0.0061803), &mut gaps) {
            return Err(Exhausted);
        }
    }
    if let Some(p) = widest_inside(gaps) {
        return Ok(Some(p));
    }
    // Nothing at those heights: the path may be parts far apart (two squares a hundred units
    // from each other, each a unit high, are missed by every one of them). Between two heights
    // that have a vertex of the polygon the same edges cross, so the middle of such a stretch
    // finds a part if there is one there; a bounded number of them, spread over the heights.
    let ys = turning_heights(edges);
    let stretches = ys.len().saturating_sub(1);
    let tries = stretches.min(MAX_HEIGHT_TRIES);
    let mut gaps: Vec<(f64, Point)> = vec![];
    for k in 0..tries {
        // Evenly spaced among the stretches, the first and the last included.
        let at = if tries > 1 { k * (stretches - 1) / (tries - 1) } else { 0 };
        if let (Some(lo), Some(hi)) = (ys.get(at), ys.get(at + 1))
            && !gaps_at((lo + hi) / 2.0, &mut gaps)
        {
            return Err(Exhausted);
        }
    }
    Ok(widest_inside(gaps))
}

/// How far the crossing of the edge `a` to `c` with a line can be off its computed `x` (of
/// `a.x + t (c.x - a.x)`): the rounding of the computation, half a unit in the last place of the sum and five
/// halves of one in `dx` for the product, and the rounding of the points the edge was made from, which moves
/// the crossing as they move: two units in the last place of the largest x the sum is made of (the x of each
/// end, and of the crossing), and the crossing's own rate of change with y, `|dx / dy|`, times a unit in the last
/// place of the largest y of the ends. A retrace along a slanted line, through points rounded each way, lies
/// within that of itself; the second term is the one that tells a line steep in x, high up in y, apart from a
/// part (its passes are a hundred times a unit in x apart at a hundred times the slope).
fn crossing_rounding(a: Point, c: Point, x: f64) -> f64 {
    let ulp = |v: f64| v.abs().next_up() - v.abs();
    let (dx, dy) = (c.x - a.x, c.y - a.y);
    let across = ulp(a.x.abs().max(c.x.abs()).max(x.abs()));
    let along = if dy == 0.0 { 0.0 } else { (dx / dy).abs() * ulp(a.y.abs().max(c.y.abs())) };
    0.5 * f64::EPSILON * (x.abs() + 5.0 * dx.abs()) + 2.0 * across + along
}

/// The heights of the vertices of the polygon `edges` where it turns, or where a run of edges that
/// follow one another starts or ends, sorted and each once. Where the polygon passes straight on
/// through a vertex, the line through the edges on either side is one, and a line at any height
/// between the turns crosses it at an x that moves evenly with the height: no gap opens or closes at
/// such a vertex. So a path that goes out and back along a line through other vertices than it went
/// out by (which do not cancel as a retrace) puts no heights here but its two ends'.
fn turning_heights(edges: &[(Point, Point)]) -> Vec<f64> {
    // `(a, b)` then `(c, d)`: the second follows on from the first, in the same direction along one line.
    // On one line to within rounding: the vertex is no further from the line through its neighbours
    // than a few units in the last place of the three points' coordinates across that line and of the
    // two edges' lengths, which is where rounding puts points on a slanted line. Across the line: each
    // coordinate weighted by how far the line's normal points along it, so that a line along y far out
    // in y is told apart at the precision of x, which rounding of y does not move a point across. An
    // angle would not do: a vertex a tenth off the line between neighbours a billion away turns the
    // edges by less than a billionth of a radian, and the bulge it makes encloses area that a line at
    // no height of the search would cross.
    let straight_on = |(a, b): (Point, Point), (c, d): (Point, Point)| {
        let (u, v) = (b - a, d - c);
        let w = u + v;
        let reach = w.hypot();
        // Each point's coordinates across `w`, times its length.
        let across = |p: Point| p.x.abs() * w.y.abs() + p.y.abs() * w.x.abs();
        let coords = across(a).max(across(b)).max(across(d));
        b == c && u.dot(v) > 0.0 && u.cross(v).abs() <= 8.0 * f64::EPSILON * (coords + (u.hypot() + v.hypot()) * reach)
    };
    let mut ys: Vec<f64> = Vec::with_capacity(edges.len());
    for (i, &edge) in edges.iter().enumerate() {
        let before = i.checked_sub(1).and_then(|j| edges.get(j)).copied();
        if !before.is_some_and(|e| straight_on(e, edge)) {
            ys.push(edge.0.y);
        }
        if !edges.get(i + 1).copied().is_some_and(|e| straight_on(edge, e)) {
            ys.push(edge.1.y);
        }
    }
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    ys
}

/// `edges` without the pairs of an edge and the same edge the other way, as many of each way as the
/// difference of their numbers leaves, in their order; and without edges of no length.
pub(super) fn without_retraces(edges: Vec<(Point, Point)>) -> Vec<(Point, Point)> {
    /// A point by the bits of its coordinates.
    type Bits = (u64, u64);
    let key = |p: Point| crate::path::point_bits(p);
    // Each edge by its ends in order: how many go from the lesser end to the greater, less how many go back.
    let mut net: HashMap<(Bits, Bits), i64> = HashMap::new();
    for &(a, c) in edges.iter().filter(|(a, c)| a != c) {
        let (ka, kc) = (key(a), key(c));
        if ka < kc {
            *net.entry((ka, kc)).or_default() += 1;
        } else {
            *net.entry((kc, ka)).or_default() -= 1;
        }
    }
    let mut kept = Vec::with_capacity(edges.len());
    for (a, c) in edges.into_iter().filter(|(a, c)| a != c) {
        let (ka, kc) = (key(a), key(c));
        let (ends, forward) = if ka < kc { ((ka, kc), true) } else { ((kc, ka), false) };
        let Some(left) = net.get_mut(&ends) else { continue };
        if forward && *left > 0 {
            *left -= 1;
            kept.push((a, c));
        } else if !forward && *left < 0 {
            *left += 1;
            kept.push((a, c));
        }
    }
    kept
}

/// The box of the ends of `edges`, if there are any.
fn bounds_of_edges(edges: &[(Point, Point)]) -> Option<kurbo::Rect> {
    let mut pts = edges.iter().flat_map(|&(a, c)| [a, c]);
    let first = pts.next()?;
    Some(pts.fold(kurbo::Rect::from_points(first, first), |r, p| r.union_pt(p)))
}

/// Most heights besides the first few that [`interior_point_within`] looks at when they find
/// nothing: the middles of this many of the stretches between the heights of the polygon's vertices.
const MAX_HEIGHT_TRIES: usize = 64;

/// Most middles of gaps [`interior_point_within`] tests against the path itself.
pub(super) const MAX_WINDING_TESTS: usize = 32;
