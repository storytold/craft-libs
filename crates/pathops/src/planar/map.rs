use super::graph::{
    Edge, Fill, Outlines, cycle_bezpath, far_end, find, half_segs, leaving_angle, origin, pin_ends, there_and_back, twin, unshear, without_spurs,
};
use super::index::{Crossings, RectIndex, SHEAR, UNSHEAR, mid};
use super::*;

/// What is left of input `shape` when the faces that hold `merged` merge on their own, with the
/// inputs that cover them marked in `cover` ([`Map::merge`]).
pub(crate) struct Rest {
    pub(crate) shape: usize,
    pub(crate) merged: Vec<Point>,
    pub(crate) cover: Vec<bool>,
}

/// The planar graph of `shapes` (in the sweep's frame) with its faces walked: what [`build`] makes
/// Live Paint faces and edges from, and what [`Map::merge`] merges faces in.
pub(crate) struct Map {
    pub(super) edges: Vec<Edge>,
    around: Vec<Vec<usize>>,
    slot_of: Vec<usize>,
    /// Bounded faces: their cycle of half-edges, path and area; and, by their bounds, which can
    /// hold a point, so finding the face around one tests only those.
    pub(super) faces: Vec<(Vec<usize>, BezPath, f64)>,
    face_index: RectIndex<usize>,
    /// Bounded faces left out as slivers ([`is_sliver`]): their cycle of half-edges and the inputs
    /// whose fill covers them (none for a gap, such as a thin cutout).
    pub(super) slivers: Vec<(Vec<usize>, Vec<usize>)>,
    /// Outer boundaries of connected pieces, and the face each one is a hole in.
    pub(super) holes: Vec<(Vec<usize>, BezPath)>,
    pub(super) face_holes: Vec<Vec<usize>>,
    pub(super) tidy: Tidy,
    /// The inputs' anchors (sorted by x) and how close results snap back onto them.
    pub(super) anchors: Vec<Point>,
    pub(super) snap: f64,
}

/// With `cover`, also finds which inputs cover each sliver, which only [`Map::merge`] reads;
/// without it the slivers' lists of inputs stay empty.
pub(crate) fn map(shapes: &[Shape], cover: bool) -> Option<Map> {
    map_with(shapes, cover, DEFAULT_PRECISION)
}

/// [`map`] whose faces and pieces are refit to `precision`, and whose slivers thinner than it are dropped.
pub(super) fn map_with(shapes: &[Shape], cover: bool, precision: f64) -> Option<Map> {
    let mut inputs: Vec<(BezPath, usize)> = Vec::new();
    for (i, s) in shapes.iter().enumerate() {
        let mut bp = BezPath::new();
        for sp in s.path.subpaths.iter().filter(|sp| sp.anchors.len() >= 2) {
            if sp.closed {
                sp.to_bezpath_into(&mut bp);
            } else {
                let mut open = BezPath::new();
                sp.to_bezpath_into(&mut open);
                there_and_back(&open, &mut bp);
            }
        }
        if !bp.elements().is_empty() {
            bp.apply_affine(SHEAR);
            inputs.push((bp, i));
        }
    }
    if inputs.is_empty() {
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
    for (v, list) in around.iter_mut().enumerate() {
        let Some(&at) = points.get(v) else { continue };
        // Half the shortest chord of the edges here: every one of them gets that far from it.
        let r = 0.5 * list.iter().filter_map(|&h| far_end(&edges, h).map(|p| p.distance(at))).fold(f64::INFINITY, f64::min);
        let mut keyed: Vec<(f64, usize)> = list.iter().map(|&h| (leaving_angle(&edges, h, at, r), h)).collect();
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
    let mut slivers: Vec<(Vec<usize>, BezPath)> = Vec::new();
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
        if area > 0.0 && !is_sliver(&bp, tidy.precision) {
            faces.push((cycle, bp, area));
        } else if area > 0.0 {
            slivers.push((cycle, bp));
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

    // The inputs that cover each sliver, tested at a point inside it.
    let fills: Vec<Fill> = if slivers.is_empty() || !cover { Vec::new() } else { shapes.iter().map(Fill::new).collect() };
    let slivers: Vec<(Vec<usize>, Vec<usize>)> = slivers
        .into_iter()
        .map(|(cycle, mut bp)| {
            if !cover {
                return (cycle, Vec::new());
            }
            bp.apply_affine(UNSHEAR);
            let sources = interior_point(&PathData::from_bezpath(&bp))
                .map(|p| fills.iter().enumerate().filter(|(_, f)| f.contains(p)).map(|(i, _)| i).collect())
                .unwrap_or_default();
            (cycle, sources)
        })
        .collect();

    // The inputs' anchors, which results land on again up to the sweep's tolerance.
    let snap = 2.0 * eps;
    let mut anchors: Vec<Point> = shapes.iter().flat_map(|s| s.path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p))).collect();
    anchors.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    Some(Map { edges, around, slot_of, faces, face_index, slivers, holes, face_holes, tidy, anchors, snap })
}

fn sweep_graph(inputs: &[(BezPath, usize)], eps: f64, precision: f64) -> Option<(Vec<Point>, Vec<Edge>)> {
    // The sweep can panic on rare near-degenerate input; that must never take the app down.
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
        let segs: Vec<Seg> = positions[idx].path.segments().map(to_seg).collect();
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
        // each face carries its own rounding of the corner (#893).
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

impl Map {
    /// The union of the faces that hold `inside` (document points, one per face), traced along the
    /// map's own edges: those between two merged faces drop out exactly, where a boolean union of
    /// the faces' outlines can leave them in when its sweep rounds their shared corners apart
    /// (#893). A sliver next to a merged face that only inputs marked in `cover` fill merges with
    /// them, so it doesn't stay as a hairline gap between them. With `rest`, a sliver goes in only
    /// when its input covers it, and not when the merge of the faces merged on their own takes it
    /// in. Outer outlines come out positive, holes negative. None when a point is in no face.
    pub(crate) fn merge(&self, inside: &[Point], cover: &[bool], rest: Option<&Rest>) -> Option<PathData> {
        let halves = 2 * self.edges.len();
        // The face on each half-edge's side; holes bound the face around them. Slivers come after
        // the faces.
        let mut face_of: Vec<Option<usize>> = vec![None; halves];
        for (i, (cycle, ..)) in self.faces.iter().enumerate() {
            let hole_cycles = self.face_holes.get(i)?.iter().filter_map(|&k| self.holes.get(k)).map(|(c, _)| c);
            for &h in std::iter::once(cycle).chain(hole_cycles).flatten() {
                *face_of.get_mut(h)? = Some(i);
            }
        }
        for (k, (cycle, _)) in self.slivers.iter().enumerate() {
            for &h in cycle {
                *face_of.get_mut(h)? = Some(self.faces.len() + k);
            }
        }
        // The smallest face around a point: faces of a piece inside a hole lie within the face around.
        let face_at = |p: Point| {
            let q = SHEAR * p;
            self.face_index
                .at(q, 0.0)
                .filter_map(|&i| self.faces.get(i).map(|f| (i, f)))
                .filter(|(_, (_, bp, _))| bp.winding(q) != 0)
                .min_by(|a, b| a.1.2.total_cmp(&b.1.2))
                .map(|(i, _)| i)
        };
        let mut picked = vec![false; self.faces.len() + self.slivers.len()];
        for &p in inside {
            *picked.get_mut(face_at(p)?)? = true;
        }
        let mut other = vec![false; self.faces.len()];
        for i in rest.iter().flat_map(|r| &r.merged).filter_map(|&p| face_at(p)) {
            if let Some(o) = other.get_mut(i) {
                *o = true;
            }
        }
        let next_to = |cycle: &[usize], marked: &[bool]| {
            cycle.iter().any(|&h| face_of.get(twin(h)).copied().flatten().is_some_and(|f| f < self.faces.len() && marked.get(f) == Some(&true)))
        };
        for (k, (cycle, sources)) in self.slivers.iter().enumerate() {
            let within = |c: &[bool]| !sources.is_empty() && sources.iter().all(|&i| c.get(i) == Some(&true));
            let ours = rest.is_none_or(|r| sources.contains(&r.shape) && !(within(&r.cover) && next_to(cycle, &other)));
            if within(cover) && ours && next_to(cycle, &picked) {
                *picked.get_mut(self.faces.len() + k)? = true;
            }
        }
        let merged = |h: usize| face_of.get(h).copied().flatten().is_some_and(|f| picked.get(f) == Some(&true));
        let boundary = |h: usize| merged(h) && !merged(twin(h));
        let mut used = vec![false; halves];
        let mut subpaths = Vec::new();
        for start in 0..halves {
            if !boundary(start) || used.get(start).copied().unwrap_or(true) {
                continue;
            }
            let mut cycle = vec![];
            let mut h = start;
            while !used.get(h).copied().unwrap_or(true) {
                if let Some(u) = used.get_mut(h) {
                    *u = true;
                }
                cycle.push(h);
                // Turn from the way back as a face walk does, past the edges inside the merged
                // faces (between two of them, or a line's loose end), to the next boundary edge.
                let back = twin(h);
                let list = self.around.get(origin(&self.edges, back)?)?;
                let k = *self.slot_of.get(back)?;
                let n = list.len();
                h = (1..=n).filter_map(|j| list.get((k + n * 2 - j) % n).copied()).find(|&g| boundary(g))?;
            }
            if h != start {
                return None;
            }
            let segs: Vec<Seg> = cycle.iter().flat_map(|&h| half_segs(&self.edges, h)).collect();
            subpaths.push(segs_to_subpath(&tidy_segments(segs, true, &self.tidy), true)?);
        }
        (!subpaths.is_empty()).then(|| unshear(PathData::new(subpaths), &self.anchors, self.snap))
    }
}

/// Faces and edges of `shapes`, and the pieces of the open paths (those with no closed subpath)
/// that lie under another path in front of them, whose edges are that path's: under a closed
/// path's outline, or along an open path in front.
pub(super) fn build(shapes: &[Shape], precision: f64) -> Option<(Vec<Region>, Vec<Shape>, Vec<Shape>)> {
    let m = map_with(shapes, false, precision)?;
    let Map { edges, around, faces, holes, face_holes, tidy, anchors, snap, .. } = &m;
    let snap = *snap;
    let fills: Vec<Fill> = shapes.iter().map(Fill::new).collect();
    let subpath = |cycle: &[usize]| {
        let segs: Vec<Seg> = cycle.iter().flat_map(|&h| half_segs(edges, h)).collect();
        segs_to_subpath(&tidy_segments(segs, true, tidy), true)
    };
    let regions = faces
        .iter()
        .zip(face_holes)
        .filter_map(|((cycle, _, _), hs)| {
            let mut subpaths = vec![subpath(cycle)?];
            subpaths.extend(hs.iter().filter_map(|&k| holes.get(k)).filter_map(|(c, _)| subpath(c)));
            let path = unshear(PathData::new(subpaths), anchors, snap);
            let sources =
                // A face finer than the default precision can need a finer outline to find a point in.
                interior_point(&path)
                    .or_else(|| (precision < DEFAULT_PRECISION).then(|| interior_point_within(&path, precision / 4.0)).flatten())
                    .map(|p| fills.iter().enumerate().filter(|(_, f)| f.contains(p)).map(|(i, _)| i).collect())
                    .unwrap_or_default();
            Some(Region { path, sources })
        })
        .collect();

    // Edges: chains of segments through vertices where nothing else meets.
    let to_shapes = |chains: Vec<(Vec<Seg>, bool, usize)>| -> Vec<Shape> {
        chains
            .into_iter()
            .filter_map(|(segs, closed, source)| {
                let sp = segs_to_subpath(&tidy_segments(segs, closed, tidy), closed)?;
                Some(Shape::new(unshear(PathData::new(vec![sp]), anchors, snap), FillRule::NonZero, shapes.get(source)?.key))
            })
            .collect()
    };
    let is_line = |i: usize| shapes.get(i).is_some_and(|s| !s.path.subpaths.iter().any(|sp| sp.closed));
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

/// A point strictly inside a filled path, well away from its edges where it can be (for re-finding
/// a Live Paint face after a rebuild).
pub fn interior_point(path: &PathData) -> Option<Point> {
    let bb = path.bounds()?;
    interior_point_within(path, (bb.width().max(bb.height()) * 1e-3).max(1e-4))
}

/// [`interior_point`] with the path's curves flattened to within `tolerance`.
fn interior_point_within(path: &PathData, tolerance: f64) -> Option<Point> {
    let bb = path.bounds()?;
    let bp = path.to_bezpath();
    let mut edges: Vec<(Point, Point)> = vec![];
    let (mut first, mut last) = (Point::ZERO, Point::ZERO);
    kurbo::flatten(bp.iter(), tolerance, |el| match el {
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
    let mut best: Option<(f64, Point)> = None;
    for f in [0.5, 0.25, 0.75, 0.125, 0.375, 0.625, 0.875, 0.0625, 0.9375] {
        // A little off the round fractions, which on art drawn to a grid fall on the height of
        // some corner, where testing the point against other paths is unreliable.
        let y = bb.y0 + bb.height() * (f + 0.0061803);
        let mut xs: Vec<f64> =
            edges.iter().filter(|(a, c)| (a.y <= y) != (c.y <= y)).map(|(a, c)| a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x)).collect();
        xs.sort_by(f64::total_cmp);
        for w in xs.windows(2) {
            let m = Point::new((w[0] + w[1]) / 2.0, y);
            let width = w[1] - w[0];
            if width > best.map_or(0.0, |b| b.0) && bp.winding(m) != 0 {
                best = Some((width, m));
            }
        }
    }
    best.map(|b| b.1)
}
