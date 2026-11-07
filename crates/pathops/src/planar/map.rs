// Carved from VectorCraft's crates/pathops (src/planar/map.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use super::graph::{
    Edge, Fill, Outlines, cycle_bezpath, far_end, find, half_segs, leaving_angle, origin, pin_ends, there_and_back, twin, unshear, without_spurs,
};
use super::index::{Crossings, RectIndex, mid, shear, shear_for};
use super::*;
use crate::path::subs;
use crate::work::{self, Work};

/// The planar graph of `shapes` (in the sweep's frame) with its faces walked: what [`build`] makes
/// the regions and edges of.
pub(crate) struct Map {
    edges: Vec<Edge>,
    around: Vec<Vec<usize>>,
    /// Bounded faces: their cycle of half-edges, path and area.
    faces: Vec<(Vec<usize>, BezPath, f64)>,
    /// Outer boundaries of connected pieces, and the face each one is a hole in.
    holes: Vec<(Vec<usize>, BezPath)>,
    face_holes: Vec<Vec<usize>>,
    tidy: Tidy,
    /// The inputs' anchors (sorted by x) and how close results snap back onto them.
    anchors: Vec<Point>,
    snap: f64,
    /// The shear's inverse: from the sweep's frame back to the document.
    back: kurbo::Affine,
}

/// The planar map of `shapes`, whose faces and pieces are refit to `precision`, and whose slivers
/// thinner than it are dropped.
fn map_with(shapes: &[Shape], precision: f64) -> Option<Map> {
    if !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return None;
    }
    let mut inputs: Vec<(BezPath, usize)> = Vec::new();
    let (forward, back) = shear(shear_for(shapes));
    for (i, s) in shapes.iter().enumerate() {
        let mut bp = BezPath::new();
        for sp in subs(&s.path).iter().filter(|sp| sp.draws_something()) {
            if sp.closed {
                sp.push_closed(&mut bp);
            } else {
                let mut open = BezPath::new();
                sp.push_open(&mut open);
                there_and_back(&open, &mut bp);
            }
        }
        if !bp.elements().is_empty() {
            bp.apply_affine(forward);
            inputs.push((bp, i));
        }
    }
    if inputs.is_empty() || !crate::budget::fits_elements(inputs.iter().map(|p| &p.0)) {
        return None;
    }
    let eps = eps_for(inputs.iter().map(|p| &p.0)).ok()?;
    snap_horizontals(&mut inputs.iter_mut().map(|p| &mut p.0).collect::<Vec<_>>(), eps);
    // Counted on the snapped geometry, which is what the sweep sees: snapping can bring boxes into contact.
    if !crate::budget::sweep_fits(inputs.iter().map(|p| &p.0)) {
        return None;
    }
    let tidy = Tidy::keeping(precision, inputs.iter().map(|p| &p.0));
    let (points, edges) = sweep_graph(&inputs, eps, precision)?;
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
        let area = bp.area();
        // A positive cycle that is a sliver (thinner than the precision) is no face and is left out.
        if area > 0.0 && !is_sliver(&bp, tidy.precision) {
            faces.push((cycle, bp, area));
        } else if area < -(tidy.precision * tidy.precision) {
            holes.push((cycle, bp));
        }
    }

    // Each hole goes in the smallest face of another piece around it.
    let face_index = RectIndex::new(faces.iter().enumerate().map(|(i, (_, bp, _))| (bp.bounding_box(), i)).collect());
    let mut face_holes: Vec<Vec<usize>> = vec![Vec::new(); faces.len()];
    for (k, (cycle, _)) in holes.iter().enumerate() {
        let Some(p) = cycle.first().and_then(|&h| origin(&edges, h)).and_then(|v| points.get(v)).copied() else { continue };
        let own = piece_of(cycle);
        let around_it = face_index
            .at(p, 0.0)
            .filter_map(|&i| faces.get(i).map(|f| (i, f)))
            .filter(|(_, (fc, bp, _))| piece_of(fc) != own && bp.winding(p) != 0)
            .min_by(|a, b| a.1.2.total_cmp(&b.1.2))
            .map(|(i, _)| i);
        if let Some(list) = around_it.and_then(|i| face_holes.get_mut(i)) {
            list.push(k);
        }
    }

    // The inputs' anchors, which results land on again up to the sweep's tolerance.
    let snap = 2.0 * eps;
    let mut anchors: Vec<Point> = shapes.iter().flat_map(|s| s.path.elements().iter().filter_map(kurbo::PathEl::end_point)).collect();
    anchors.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    Some(Map { edges, around, faces, holes, face_holes, tidy, anchors, snap, back })
}

/// The points where three or more edges of the planar map of `shapes` meet, by x and then y. Open
/// paths are edges of the map (traced there and back), so where one crosses another path is a
/// junction, which it is not in the arrangement of the filled areas, where the way there and the
/// way back cancel.
pub(crate) fn junctions(shapes: &[Shape], precision: f64) -> Option<Vec<Point>> {
    let m = crate::path::guarded(|| map_with(shapes, precision)).flatten()?;
    let mut out: Vec<Point> = Vec::new();
    for list in m.around.iter().filter(|l| l.len() >= 3) {
        let Some(&h) = list.first() else { continue };
        let e = m.edges.get(h / 2)?;
        // Every edge at a vertex starts or ends exactly on it.
        let at = if h.is_multiple_of(2) { e.segs.first()?.c.p0 } else { e.segs.last()?.c.p3 };
        out.push(m.back * at);
    }
    out.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    Some(out)
}

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

/// Faces and edges of `shapes`, and the pieces of the open paths (those with no closed subpath)
/// that lie under another path in front of them, whose edges are that path's: under a closed
/// path's outline, or along an open path in front.
pub(super) fn build(shapes: &[Shape], precision: f64) -> Option<(Vec<Region>, Vec<Shape>, Vec<Shape>)> {
    // The sweep can panic on rare near-degenerate input; that is no map.
    let m = crate::path::guarded(|| map_with(shapes, precision)).flatten()?;
    let Map { edges, around, faces, holes, face_holes, tidy, anchors, snap, back } = &m;
    let snap = *snap;
    let fills: Vec<Fill> = shapes.iter().map(Fill::new).collect();
    // The fills whose bounds hold a point, in the order of the shapes: not every fill is tested for every face.
    let fill_index = RectIndex::new(fills.iter().enumerate().map(|(i, f)| (f.bounds, i)).collect());
    let subpath = |cycle: &[usize]| {
        let segs: Vec<Seg> = cycle.iter().flat_map(|&h| half_segs(edges, h)).collect();
        segs_to_bezpath(&tidy_segments(segs, true, tidy), true)
    };
    let regions = faces
        .iter()
        .zip(face_holes)
        .filter_map(|((cycle, _, _), hs)| {
            let mut path = subpath(cycle)?;
            for sp in hs.iter().filter_map(|&k| holes.get(k)).filter_map(|(c, _)| subpath(c)) {
                path.extend(sp.iter());
            }
            let path = unshear(path, anchors, snap, *back);
            let sources =
                // A face finer than the default precision can need a finer outline to find a point in.
                interior_point(&path)
                    .or_else(|| (precision < DEFAULT_PRECISION).then(|| interior_point_within(&path, precision / 4.0)).flatten())
                    .map(|p| fill_index.at(p, 0.0).copied().filter(|&i| fills.get(i).is_some_and(|f| f.contains(p))).collect())
                    .unwrap_or_default();
            Some(Region { path, sources })
        })
        .collect();

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
    let lines: Vec<bool> = shapes.iter().map(|s| !crate::path::has_closed_subpath(&s.path)).collect();
    let is_line = |i: usize| lines.get(i) == Some(&true);
    // An edge is the front-most input's. Every other open path along it has a piece there too.
    let edge_shapes = to_shapes(chains_by(edges, around, |e| vec![e.source]));
    let under = to_shapes(chains_by(edges, around, |e| e.sources.iter().rev().copied().filter(|&i| i != e.source && is_line(i)).collect()));
    Some((regions, edge_shapes, under))
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
/// with no area, or one whose points are not finite.
pub fn interior_point(path: &BezPath) -> Option<Point> {
    let bb = finite_bounds(path)?;
    interior_point_within(path, (bb.width().max(bb.height()) * 1e-3).max(1e-4))
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
    let bb = finite_bounds(path)?;
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return None;
    }
    let bp = path;
    let mut edges: Vec<(Point, Point)> = vec![];
    let (mut first, mut last) = (Point::ZERO, Point::ZERO);
    crate::path::flatten(bp.iter(), tolerance, |el| match el {
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
    if last != first {
        edges.push((last, first));
    }
    // The gaps between consecutive crossings of the polygon with a line at each of a few heights,
    // each with the width the polygon has at its middle: the widest where the polygon winds.
    let gaps_at = |y: f64, min_width: f64, gaps: &mut Vec<(f64, Point)>| {
        work::add(Work::InteriorHeights, 1);
        // Each crossing's x and which way the edge goes, so that the winding number in a gap is
        // the sum of the crossings before it (one sort and one pass, not a winding per gap).
        let mut xs: Vec<(f64, i32)> = edges
            .iter()
            .filter(|(a, c)| (a.y <= y) != (c.y <= y))
            .map(|(a, c)| (a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x), if c.y > a.y { 1 } else { -1 }))
            .collect();
        xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        for w in xs.windows(2) {
            winding += w[0].1;
            let width = w[1].0 - w[0].0;
            if winding != 0 && width > min_width {
                gaps.push((width, Point::new((w[0].0 + w[1].0) / 2.0, y)));
            }
        }
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
                bp.winding(*m) != 0
            })
            .map(|(_, m)| m)
    };
    let mut gaps: Vec<(f64, Point)> = vec![];
    for f in [0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875, 0.0625, 0.9375] {
        // A little off the round fractions, which on art drawn to a grid fall on the height of
        // some corner, where testing the point against other paths is unreliable.
        gaps_at(bb.y0 + bb.height() * (f + 0.0061803), 0.0, &mut gaps);
    }
    if let Some(p) = widest_inside(gaps) {
        return Some(p);
    }
    // Nothing at those heights: the path may be parts far apart (two squares a hundred units
    // from each other, each a unit high, are missed by every one of them). Between two heights
    // that have a vertex of the polygon the same edges cross, so the middle of such a stretch
    // finds a part if there is one there; a bounded number of them, spread over the heights.
    let mut ys: Vec<f64> = edges.iter().flat_map(|(a, c)| [a.y, c.y]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    // A gap thinner than the rounding of the crossings is where edges retrace each other, not a part.
    let min_width = 1e-9 * bb.width().max(bb.height());
    let stretches = ys.len().saturating_sub(1);
    let tries = stretches.min(MAX_HEIGHT_TRIES);
    let mut gaps: Vec<(f64, Point)> = vec![];
    for k in 0..tries {
        // Evenly spaced among the stretches, the first and the last included.
        let at = if tries > 1 { k * (stretches - 1) / (tries - 1) } else { 0 };
        if let (Some(lo), Some(hi)) = (ys.get(at), ys.get(at + 1)) {
            gaps_at((lo + hi) / 2.0, min_width, &mut gaps);
        }
    }
    widest_inside(gaps)
}

/// Most heights besides the first few that [`interior_point_within`] looks at when they find
/// nothing: the middles of this many of the stretches between the heights of the polygon's vertices.
const MAX_HEIGHT_TRIES: usize = 64;

/// Most middles of gaps [`interior_point_within`] tests against the path itself.
pub(super) const MAX_WINDING_TESTS: usize = 32;
