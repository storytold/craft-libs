use super::map::build;
use super::*;

/// The Live Paint planar map of `shapes` (back → front).
///
/// Faces are the bounded areas the paths enclose, holes included, each with the inputs whose fill
/// covers it (ascending; empty for an area only enclosed by paths). Edges are the paths split
/// wherever they meet another path, keyed by the input they came from (the front-most where
/// inputs share one). Empty when the paths can't be swept (non-finite or too degenerate).
pub fn live_paint(shapes: &[Shape]) -> (Vec<Region>, Vec<Shape>) {
    build(shapes, DEFAULT_PRECISION).map(|(faces, edges, _)| (faces, edges)).unwrap_or_default()
}

/// [`live_paint`] with Gap Detection: the faces are those of `shapes` with a line across each gap
/// [`find_gaps`] finds up to `max_gap`, so paint stops at the gaps, and the edges are those of
/// `shapes` alone (the lines add none). Also returns the gaps.
pub fn live_paint_with_gaps(shapes: &[Shape], max_gap: f64) -> (Vec<Region>, Vec<Shape>, Vec<[Point; 2]>) {
    let gaps = find_gaps(shapes, max_gap);
    if gaps.is_empty() {
        let (faces, edges) = live_paint(shapes);
        return (faces, edges, gaps);
    }
    let mut closed = shapes.to_vec();
    let lines = PathData::new(gaps.iter().map(|g| SubPath::polyline(g, false)).collect());
    closed.push(Shape::new(lines, FillRule::NonZero, u64::MAX));
    let (faces, _) = live_paint(&closed);
    let (_, edges) = live_paint(shapes);
    (faces, edges, gaps)
}

/// How many times longer than a gap the way between its two ends along the paths must be for it to
/// count: a loose end that crosses a path just before it (an overshoot), or a path that curls back
/// close to its own end (a hook), leaves no gap there.
const GAP_DETOUR: f64 = 8.0;

/// How exactly [`find_gaps`] measures lengths along the paths.
const GAP_ARC_ACCURACY: f64 = 1e-4;

/// How many points along a loose end's own segment [`find_gaps`] tries.
const GAP_SAMPLES: u32 = 64;

/// One subpath's segments for [`find_gaps`], with the length along it where each starts.
struct Track {
    segs: Vec<PathSeg>,
    /// The length at the start of each segment, then the whole length.
    at: Vec<f64>,
    closed: bool,
    bounds: kurbo::Rect,
}

impl Track {
    fn new(sp: &SubPath) -> Option<Self> {
        if sp.anchors.len() < 2 || !sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()) {
            return None;
        }
        let mut bp = BezPath::new();
        sp.to_bezpath_into(&mut bp);
        let segs: Vec<PathSeg> = bp.segments().collect();
        let mut at = Vec::with_capacity(segs.len() + 1);
        let mut len = 0.0;
        for s in &segs {
            at.push(len);
            len += s.arclen(GAP_ARC_ACCURACY);
        }
        at.push(len);
        (!segs.is_empty()).then(|| Self { bounds: bp.bounding_box(), segs, at, closed: sp.closed })
    }

    fn len(&self) -> f64 {
        self.at.last().copied().unwrap_or(0.0)
    }

    /// The length along the subpath to parameter `t` of segment `k`.
    fn pos(&self, k: usize, t: f64) -> f64 {
        let Some(s) = self.segs.get(k) else { return self.len() };
        self.at.get(k).copied().unwrap_or(0.0) + s.subsegment(0.0..t).arclen(GAP_ARC_ACCURACY)
    }

    /// The length along the subpath between two places on it (the shorter way round a closed one).
    fn along(&self, a: f64, b: f64) -> f64 {
        let d = (a - b).abs();
        if self.closed { d.min(self.len() - d) } else { d }
    }
}

/// Do two boxes (lines' boxes included, which have no area) touch or overlap?
fn boxes_meet(a: kurbo::Rect, b: kurbo::Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Where track `o`, within `reach` along it of `from`, crosses the segments `near` of track `q`:
/// (length along `o`, length along `q`) of each crossing.
fn crossings(o: &Track, from: f64, reach: f64, q: &Track, near: impl Iterator<Item = usize>) -> Vec<(f64, f64)> {
    let near: Vec<(usize, &PathSeg)> = near.filter_map(|j| Some((j, q.segs.get(j)?))).collect();
    let mut out = vec![];
    // `first` is the first segment that ends within reach.
    let first = o.at.partition_point(|&a| a < from - reach).saturating_sub(1);
    for (k, s) in o.segs.iter().enumerate().skip(first) {
        let (Some(&a0), Some(&a1)) = (o.at.get(k), o.at.get(k + 1)) else { continue };
        if a0 > from + reach {
            break;
        }
        if a1 < from - reach || !boxes_meet(s.bounding_box(), q.bounds) {
            continue;
        }
        // A curve is tried as 16 chords.
        let n = if matches!(s, PathSeg::Line(_)) { 1 } else { 16 };
        let mut prev = s.start();
        for i in 1..=n {
            let (t0, t1) = (f64::from(i - 1) / f64::from(n), f64::from(i) / f64::from(n));
            let p = s.eval(t1);
            let chord = kurbo::Line::new(prev, p);
            let cb = chord.bounding_box();
            for &(j, qs) in &near {
                if !boxes_meet(qs.bounding_box(), cb) {
                    continue;
                }
                for x in qs.intersect_line(chord) {
                    out.push((o.pos(k, t0 + x.line_t * (t1 - t0)), q.pos(j, x.segment_t)));
                }
            }
            prev = p;
        }
    }
    out
}

/// Most grid cells one segment is listed in by [`SegGrid`]; a longer one is tried for every point.
const GAP_GRID_SPAN: i64 = 1024;

/// The farthest cell from the grid's origin, on each axis, that [`SegGrid`] numbers a point in.
const GAP_GRID_LIMIT: f64 = 1_048_576.0;

/// The segments of [`find_gaps`]' tracks by the cells of a square grid that their bounds cover.
struct SegGrid {
    origin: Point,
    cell: f64,
    /// (track, segment) by cell.
    cells: HashMap<(i64, i64), Vec<(usize, usize)>>,
    /// Segments whose bounds cover more than [`GAP_GRID_SPAN`] cells, near every point.
    wide: Vec<(usize, usize)>,
}

impl SegGrid {
    fn new(tracks: &[Track], origin: Point, cell: f64) -> Self {
        let mut grid = Self { origin, cell, cells: HashMap::new(), wide: vec![] };
        for (j, t) in tracks.iter().enumerate() {
            for (k, s) in t.segs.iter().enumerate() {
                let b = s.bounding_box();
                let ((x0, y0), (x1, y1)) = (grid.cell_of(Point::new(b.x0, b.y0)), grid.cell_of(Point::new(b.x1, b.y1)));
                if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > GAP_GRID_SPAN {
                    grid.wide.push((j, k));
                    continue;
                }
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        grid.cells.entry((x, y)).or_default().push((j, k));
                    }
                }
            }
        }
        grid
    }

    /// The cell `p` is in, counted from the origin (at most [`GAP_GRID_LIMIT`] cells away on each
    /// axis).
    fn cell_of(&self, p: Point) -> (i64, i64) {
        let at = |v: f64, o: f64| ((v - o) / self.cell).floor().clamp(-GAP_GRID_LIMIT, GAP_GRID_LIMIT) as i64;
        (at(p.x, self.origin.x), at(p.y, self.origin.y))
    }

    /// The segments whose bounds come within `r` of `p` (and some farther ones), each once, in
    /// track order.
    fn near(&self, p: Point, r: f64) -> Vec<(usize, usize)> {
        let mut out = self.wide.clone();
        let ((x0, y0), (x1, y1)) = (self.cell_of(Point::new(p.x - r, p.y - r)), self.cell_of(Point::new(p.x + r, p.y + r)));
        for x in x0..=x1 {
            for y in y0..=y1 {
                out.extend(self.cells.get(&(x, y)).into_iter().flatten());
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The point a gap from the loose end `e` of track `own` (at `from` along it) closes to: the
/// nearest point of any path within `max_gap` of it, leaving out points the paths already join to
/// `e` along a way less than [`GAP_DETOUR`] times as long as the gap. None when there is no such
/// point, or when `e` lies on a path (it is no loose end).
fn gap_from(tracks: &[Track], grid: &SegGrid, own: usize, e: Point, from: f64, max_gap: f64, eps: f64) -> Option<Point> {
    use kurbo::ParamCurveNearest;
    let o = tracks.get(own)?;
    let reach = GAP_DETOUR * max_gap;
    let mut best: Option<(f64, Point)> = None;
    // `around` holds the segments a way of up to `reach` from `e` can cross, by track (grouped
    // once: filtering the whole list again for each track near `e` made clusters of parallel
    // paths cubic), and `crossed` the crossings with each track.
    let mut around: Option<HashMap<usize, Vec<usize>>> = None;
    let mut crossed: HashMap<usize, Vec<(f64, f64)>> = HashMap::new();
    for (j, k) in grid.near(e, max_gap) {
        let Some(q) = tracks.get(j) else { continue };
        let Some(s) = q.segs.get(k) else { continue };
        let b = s.bounding_box();
        if e.x < b.x0 - max_gap || e.x > b.x1 + max_gap || e.y < b.y0 - max_gap || e.y > b.y1 + max_gap {
            continue;
        }
        let near = s.nearest(e, 1e-9);
        let d = near.distance_sq.sqrt();
        if d > max_gap {
            continue;
        }
        let at = q.pos(k, near.t);
        let way = if j == own {
            o.along(from, at)
        } else {
            let xs = crossed.entry(j).or_insert_with(|| {
                let by_track = around.get_or_insert_with(|| {
                    let mut by_track: HashMap<usize, Vec<usize>> = HashMap::new();
                    for (t, k) in grid.near(e, reach + max_gap) {
                        by_track.entry(t).or_default().push(k);
                    }
                    by_track
                });
                crossings(o, from, reach, q, by_track.get(&j).into_iter().flatten().copied())
            });
            xs.iter().map(|&(a, b)| o.along(from, a) + q.along(b, at)).fold(f64::INFINITY, f64::min)
        };
        if d <= eps {
            // On another path, or on its own away from the end: the end is joined there.
            if j != own || way > 4.0 * eps.max(GAP_ARC_ACCURACY) {
                return None;
            }
            // The end's own segment: a curve that comes back round can still pass near it.
            for i in 0..=GAP_SAMPLES {
                let t = f64::from(i) / f64::from(GAP_SAMPLES);
                let p = s.eval(t);
                let d = p.distance(e);
                if d > eps && d <= max_gap && o.along(from, q.pos(k, t)) > GAP_DETOUR * d && best.is_none_or(|b| d < b.0) {
                    best = Some((d, p));
                }
            }
            continue;
        }
        if way > GAP_DETOUR * d && best.is_none_or(|b| d < b.0) {
            best = Some((d, s.eval(near.t)));
        }
    }
    best.map(|b| b.1)
}

/// The gaps Live Paint's Gap Detection closes in `shapes`: from each loose end of an open subpath
/// (an end that lies on no path) to the nearest point of any path, its own included, at most
/// `max_gap` away, unless the paths already join the two along a way less than [`GAP_DETOUR`] times
/// the gap's length. Each gap is a pair of points: the loose end, then where it closes to. A gap
/// between two loose ends that close to each other is listed once.
pub fn find_gaps(shapes: &[Shape], max_gap: f64) -> Vec<[Point; 2]> {
    if !(max_gap.is_finite() && max_gap > 0.0) {
        return vec![];
    }
    let tracks: Vec<Track> = shapes.iter().flat_map(|s| &s.path.subpaths).filter_map(Track::new).collect();
    let eps = (max_gap * 1e-4).max(1e-6);
    // The grid cells are as wide as the gap, or 1/4096 of the art where that is wider.
    let bounds = tracks.iter().map(|t| t.bounds).reduce(|a, b| a.union(b)).unwrap_or_default();
    let grid = SegGrid::new(&tracks, bounds.origin(), max_gap.max(bounds.width().max(bounds.height()) / 4096.0));
    let mut gaps: Vec<[Point; 2]> = vec![];
    // `by_end` lists the gaps by the cell of their loose end.
    let mut by_end: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, o) in tracks.iter().enumerate() {
        let (Some(first), Some(last)) = (o.segs.first(), o.segs.last()) else { continue };
        // An open subpath that ends where it starts encloses its area already.
        if o.closed || first.start().distance(last.end()) <= eps {
            continue;
        }
        for (e, from) in [(first.start(), 0.0), (last.end(), o.len())] {
            let Some(to) = gap_from(&tracks, &grid, i, e, from, max_gap, eps) else { continue };
            let (cx, cy) = grid.cell_of(to);
            let listed = (cx - 1..=cx + 1)
                .flat_map(|x| (cy - 1..=cy + 1).map(move |y| (x, y)))
                .filter_map(|c| by_end.get(&c))
                .flatten()
                .filter_map(|&g| gaps.get(g))
                .any(|g| g[0].distance(to) <= eps && g[1].distance(e) <= eps);
            if !listed {
                by_end.entry(grid.cell_of(e)).or_default().push(gaps.len());
                gaps.push([e, to]);
            }
        }
    }
    gaps
}
