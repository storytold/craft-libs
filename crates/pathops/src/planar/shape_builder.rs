use super::live_paint::find_gaps;
use super::map::build;
use super::*;

/// Most segments the Shape Builder sweeps into its planar map (open paths as edges, the gaps closed
/// paths enclose, outline edges); past this only the filled areas count. The map costs about what
/// the filled areas' arrangement does (both near linear in the input; the cap was 4096 while
/// finding each edge's source scanned every segment, #937), so this only bounds the time a hover
/// can take on art far past what a drawing holds.
pub const SHAPE_BUILDER_MAX_SEGMENTS: usize = 65_536;

/// The Shape Builder's arrangement: see [`shape_builder`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuilderArrangement {
    pub regions: Vec<Region>,
    /// The pieces of the paths with no closed subpath, keyed by input index.
    pub lines: Vec<Shape>,
    /// The pieces of the other paths' outlines, keyed by input index (the front-most where inputs
    /// share one); only when asked for.
    pub edges: Vec<Shape>,
    /// The pieces of the paths with no closed subpath that lie under the outline of a closed path
    /// in front of them, or along another such path in front of them, keyed by input index. No
    /// piece of theirs is in `lines`; there the edge is the path's in front (in `edges` when it is
    /// a closed path's and they were asked for, else in `lines`).
    pub lines_under: Vec<Shape>,
}

/// Does any of `shapes` have an open subpath (a cutting edge for the Shape Builder)?
pub fn has_open_path(shapes: &[Shape]) -> bool {
    shapes.iter().flat_map(|s| &s.path.subpaths).any(|sp| !sp.closed && sp.anchors.len() >= 2)
}

/// Shape Builder regions of `shapes` (back → front), the pieces of their open paths and, with
/// `edges`, the pieces of their outlines (what erasing an edge deletes).
///
/// Closed paths bound regions with their filled areas, as in [`regions`](crate::regions), and the
/// gaps they enclose that nothing fills are regions too (empty `sources`), such as the middle of
/// four circles that touch (#1691). Open paths (the caller closes the ones whose fill should count)
/// are cutting edges: a line across a shape splits it, lines around an area enclose a region
/// nothing fills, and each path with no closed subpath comes back cut wherever another path meets
/// it, as pieces keyed by its index in `shapes`. Outlines are cut the same way. With more than
/// [`SHAPE_BUILDER_MAX_SEGMENTS`] segments or a map that can't be built, only the filled areas
/// count and there are no pieces.
pub fn shape_builder(shapes: &[Shape], edges: bool) -> BuilderArrangement {
    shape_builder_with_gap(shapes, edges, 0.0)
}

/// Most distance (document units) a Shape Builder gap may span.
pub const SHAPE_BUILDER_MAX_GAP: f64 = 72.0;

/// Original inputs and the shared detector's temporary, unfilled bridges. Keep the originals'
/// order and keys so only their split pieces can be returned as art.
pub(crate) fn gap_inputs(shapes: &[Shape], gap: f64) -> Vec<Shape> {
    let mut closed = shapes.to_vec();
    let segments: usize = shapes.iter().flat_map(|s| &s.path.subpaths).map(|sp| sp.segment_count()).sum();
    if segments <= SHAPE_BUILDER_MAX_SEGMENTS && gap.is_finite() && gap > 0.0 && has_open_path(shapes) {
        let gaps = find_gaps(shapes, gap.min(SHAPE_BUILDER_MAX_GAP));
        if !gaps.is_empty() {
            let path = PathData::new(gaps.iter().map(|g| SubPath::polyline(g, false)).collect());
            closed.push(Shape::new(path, FillRule::NonZero, u64::MAX));
        }
    }
    closed
}

/// [`shape_builder`] with Gap Detection: open paths whose ends come within `gap` (document units)
/// of another path are bridged for the regions they bound, so a region left open by a small gap is
/// found. A `gap` of 0 is [`shape_builder`]. The bridges are in no region's sources and in no piece
/// of `lines` or `edges`.
pub fn shape_builder_with_gap(shapes: &[Shape], edges: bool, gap: f64) -> BuilderArrangement {
    builder(shapes, edges, gap, DEFAULT_PRECISION)
}

/// [`shape_builder_with_gap`] refitting its results to `precision` (Pathfinder Options › Precision).
pub(crate) fn builder(shapes: &[Shape], edges: bool, gap: f64, precision: f64) -> BuilderArrangement {
    let has_open = has_open_path(shapes);
    let segments: usize = shapes.iter().flat_map(|s| &s.path.subpaths).map(|sp| sp.segment_count()).sum();
    let planar = (segments <= SHAPE_BUILDER_MAX_SEGMENTS)
        .then(|| {
            let keyed: Vec<Shape> = shapes.iter().enumerate().map(|(i, s)| Shape::new(s.path.clone(), s.rule, i as u64)).collect();
            if has_open && gap.is_finite() && gap > 0.0 {
                let (faces, pieces, under) = build(&gap_inputs(&keyed, gap), precision).unwrap_or_default();
                // Gap targets split original outlines too, but the bridges themselves aren't art.
                let art = |p: &Shape| p.key < shapes.len() as u64;
                (faces, pieces.into_iter().filter(art).collect(), under.into_iter().filter(art).collect())
            } else {
                build(&keyed, precision).unwrap_or_default()
            }
        })
        .filter(|(faces, pieces, _)| !faces.is_empty() || !pieces.is_empty());
    let Some((faces, pieces, under)) = planar else {
        return BuilderArrangement { regions: filled_regions(shapes, precision), ..Default::default() };
    };
    let is_line = |i: u64| shapes.get(i as usize).is_some_and(|s| !s.path.subpaths.iter().any(|sp| sp.closed));
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

/// The regions of the shapes' filled areas. A shape that fills nothing (open paths that enclose no
/// area, such as cutting lines) is left out, so a flood of them cannot push the arrangement past its
/// size budget and take the filled shapes down with it; the regions keep the original indices.
fn filled_regions(shapes: &[Shape], precision: f64) -> Vec<crate::Region> {
    let fills = |s: &Shape| s.path.subpaths.iter().any(|sp| sp.closed || encloses_area(sp));
    let keep: Vec<usize> = (0..shapes.len()).filter(|&i| shapes.get(i).is_some_and(fills)).collect();
    if keep.len() == shapes.len() {
        return crate::pathfinder::regions_with(shapes, precision);
    }
    let kept: Vec<Shape> = keep.iter().filter_map(|&i| shapes.get(i).cloned()).collect();
    crate::pathfinder::regions_with(&kept, precision)
        .into_iter()
        .map(|mut r| {
            r.sources = r.sources.iter().filter_map(|&s| keep.get(s).copied()).collect();
            r
        })
        .collect()
}

/// Does `sp`, closed, enclose an area (rather than retrace itself)? The Shape Builder closes the
/// open subpaths of filled paths that do.
pub fn encloses_area(sp: &SubPath) -> bool {
    let mut closed = sp.clone();
    closed.closed = true;
    let mut bp = BezPath::new();
    closed.to_bezpath_into(&mut bp);
    let size = bp.bounding_box().size();
    bp.area().abs() > 1e-9 * (size.width * size.width + size.height * size.height)
}

/// `path` without `piece`, a run along one of its subpaths between two points (a Shape Builder
/// edge): a closed subpath opens there, an open one splits in two (or loses an end), and a piece
/// that runs all the way round takes its subpath. With `fill_closes`, open subpaths that enclose an
/// area ([`encloses_area`]) run on round an invisible closing edge, as the Shape Builder sees a
/// filled path: a piece may run over it, and it never comes back as a visible segment. None when
/// the piece doesn't lie on `path` within `tol`.
pub fn cut_out(path: &PathData, piece: &SubPath, fill_closes: bool, tol: f64) -> Option<PathData> {
    let n = piece.segment_count();
    if n == 0 {
        return None;
    }
    // A point well inside the piece: the middle of its longest segment.
    let arclen = |k: usize| kurbo::ParamCurveArclen::arclen(&piece.segment(k), 1e-3);
    let mid = (0..n).map(|k| (arclen(k), k)).max_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, k)| piece.segment(k).eval(0.5))?;
    let (a, b) = (piece.anchors.first()?.p, piece.anchors.last()?.p);
    let mut bp = BezPath::new();
    piece.to_bezpath_into(&mut bp);
    if bp.perimeter(1e-6) <= tol {
        // A speck (where paths touch): there's nothing to cut.
        return None;
    }
    // A piece that ends where it starts runs all the way round.
    let whole = piece.closed || a.distance(b) <= tol;
    // Each subpath as the piece may run along it: round its closing edge when it has one.
    let ring = |sp: &SubPath| SubPath { anchors: sp.anchors.clone(), closed: sp.closed || (fill_closes && encloses_area(sp)) };
    let (si, geom, um) = path
        .subpaths
        .iter()
        .enumerate()
        .filter_map(|(i, sp)| {
            let g = ring(sp);
            let (u, d) = nearest_u(&g, mid)?;
            (d <= tol).then_some((i, g, u, d))
        })
        .min_by(|x, y| x.3.total_cmp(&y.3))
        .map(|(i, g, u, _)| (i, g, u))?;
    let len = geom.segment_count() as f64;
    let real = path.subpaths.get(si)?.segment_count() as f64;
    let at = |p: Point| nearest_u(&geom, p).filter(|(_, d)| *d <= tol).map(|(u, _)| u);
    // The parameter spans of `geom` that stay (unrolled past the end of a ring).
    let keep: Vec<(f64, f64)> = if whole {
        Vec::new()
    } else if geom.closed {
        let (ua, ub) = (at(a)?, at(b)?);
        let fwd = |from: f64, to: f64| (to - from).rem_euclid(len);
        // The piece runs from one end through `mid` to the other; what stays runs on round.
        let (start, end) = if fwd(ua, um) <= fwd(ua, ub) { (ub, ua) } else { (ua, ub) };
        let rest = fwd(start, end);
        if rest <= 1e-9 || len - rest <= 1e-9 {
            Vec::new()
        } else {
            // Where the ring closes with an invisible edge (`real..len`), what stays stops at it.
            let parts: &[(f64, f64)] = if real >= len { &[(0.0, 2.0 * len)] } else { &[(0.0, real), (len, len + real)] };
            parts.iter().map(|&(lo, hi)| (start.max(lo), (start + rest).min(hi))).filter(|(x, y)| y - x > 1e-9).collect()
        }
    } else {
        let (ua, ub) = (at(a)?, at(b)?);
        let (lo, hi) = (ua.min(ub), ua.max(ub));
        if !(lo - 1e-9..=hi + 1e-9).contains(&um) {
            return None;
        }
        [(0.0, lo), (hi, len)].into_iter().filter(|(x, y)| y - x > 1e-9).collect()
    };
    let mut subpaths: Vec<SubPath> = path.subpaths.iter().take(si).cloned().collect();
    subpaths.extend(keep.into_iter().filter_map(|(u0, u1)| span(&geom, u0, u1)).map(|sp| snap_ends(sp, [a, b], tol)));
    subpaths.extend(path.subpaths.iter().skip(si + 1).cloned());
    Some(PathData::new(subpaths))
}

/// The run of `piece` (a Shape Builder edge or line piece) around the point of it nearest `at`,
/// from the nearest of `stops` before that point to the nearest after it, open. Stops are the
/// points within `tol` of the piece, such as the anchors of the path it lies on; an open piece's
/// own ends bound the run where no stop comes first. None when the piece has no segments, or when
/// it is closed and fewer than two different stops lie on it (the run would be all of it).
pub fn run_around(piece: &SubPath, stops: &[Point], at: Point, tol: f64) -> Option<SubPath> {
    let n = piece.segment_count();
    if n == 0 {
        return None;
    }
    let len = n as f64;
    let (u, _) = nearest_u(piece, at)?;
    // The stops by x, so that each segment measures only the stops within its bounds.
    let mut by_x: Vec<Point> = stops.iter().copied().filter(|s| s.x.is_finite() && s.y.is_finite()).collect();
    by_x.sort_by(|a, b| a.x.total_cmp(&b.x));
    // The parameters of the stops within `tol` of segment `j` whose nearest point on the whole
    // piece lies on it (a ring's segments repeat past either end, `len` apart).
    let on = |j: isize| -> Vec<f64> {
        use kurbo::ParamCurveNearest;
        let seg = piece.segment(j.rem_euclid(n as isize) as usize);
        let b = kurbo::Shape::bounding_box(&seg).inflate(tol, tol);
        let near = by_x.get(by_x.partition_point(|s| s.x < b.x0)..).unwrap_or_default();
        near.iter()
            .take_while(|s| s.x <= b.x1)
            .filter(|s| s.y >= b.y0 && s.y <= b.y1)
            .filter_map(|s| {
                let m = seg.nearest(*s, 1e-9);
                let d = m.distance_sq.sqrt();
                (d <= tol && nearest_u(piece, *s).is_some_and(|(_, nearest)| d <= nearest + 1e-9)).then_some(j as f64 + m.t)
            })
            .collect()
    };
    // From the segment under `at`, the walk goes back to the nearest stop at or before it and on
    // to the nearest stop after it; an open piece's ends bound it.
    let k0 = (u.floor().max(0.0) as isize).min(n as isize - 1);
    let (first, last) = if piece.closed { (k0 - n as isize, k0 + n as isize) } else { (0, n as isize - 1) };
    let lo = (first..=k0).rev().find_map(|j| on(j).into_iter().filter(|&x| x <= u).max_by(f64::total_cmp));
    let lo = match lo {
        Some(lo) => lo,
        None if !piece.closed => 0.0,
        None => return None,
    };
    let after = u.max(lo + 1e-9);
    let hi = (k0..=last).find_map(|j| on(j).into_iter().filter(|&x| x > after).min_by(f64::total_cmp));
    let hi = match hi {
        Some(hi) => hi,
        None if !piece.closed => len,
        None => return None,
    };
    // On a ring, a lone stop would make the run all of it.
    if piece.closed && hi - lo >= len - 1e-9 {
        return None;
    }
    // A run that passes the ring's start begins before its end and is unrolled past it.
    let (lo, hi) = if lo < 0.0 { (lo + len, hi + len) } else { (lo, hi) };
    let run = span(piece, lo, hi)?;
    // The run's ends go exactly on the stops they end at.
    let stop_at =
        |q: Point| stops.iter().copied().filter(|s| s.distance(q) <= tol).min_by(|a, b| a.distance(q).total_cmp(&b.distance(q))).unwrap_or(q);
    let ends = [run.anchors.first()?.p, run.anchors.last()?.p].map(stop_at);
    Some(snap_ends(run, ends, tol))
}

/// The point of `sp` nearest `p`, as a parameter `segment + t`, with its distance.
fn nearest_u(sp: &SubPath, p: Point) -> Option<(f64, f64)> {
    use kurbo::ParamCurveNearest;
    #[cfg(test)]
    super::index::note_work(sp.segment_count());
    (0..sp.segment_count())
        .map(|k| {
            let n = sp.segment(k).nearest(p, 1e-9);
            (k as f64 + n.t, n.distance_sq.sqrt())
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
}

/// `sp` with its end anchors within `tol` of one of `to` put exactly on it (where the planar map
/// put the junction a piece ends at), handles moving with them.
fn snap_ends(mut sp: SubPath, to: [Point; 2], tol: f64) -> SubPath {
    let last = sp.anchors.len().saturating_sub(1);
    for i in [0, last] {
        if let Some(an) = sp.anchors.get_mut(i)
            && let Some(q) = to.iter().find(|q| q.distance(an.p) <= tol)
        {
            let d = *q - an.p;
            an.p = *q;
            an.h_in += d;
            an.h_out += d;
        }
    }
    sp
}

/// The run of `sp` from parameter `u0` to `u1` (`u0 < u1`, unrolled past the end of a closed one),
/// open.
fn span(sp: &SubPath, u0: f64, u1: f64) -> Option<SubPath> {
    let n = sp.segment_count();
    if n == 0 || !(u0.is_finite() && u1.is_finite()) {
        return None;
    }
    let first = u0.floor().max(0.0) as usize;
    let last = (u1.ceil().max(0.0) as usize).min(first + 2 * n);
    let segs: Vec<Seg> = (first..last)
        .filter_map(|k| {
            let (t0, t1) = ((u0 - k as f64).clamp(0.0, 1.0), (u1 - k as f64).clamp(0.0, 1.0));
            (t1 - t0 > 1e-12).then(|| Seg { c: sp.segment(k % n).subsegment(t0..t1), line: sp.segment_is_line(k % n) })
        })
        .collect();
    segs_to_subpath(&segs, false)
}
