//! Stencil Bridges: the art a stencil is cut from, with strips cut out of it so that every island
//! of the stencil sheet stays attached.
//!
//! The art is the area the stencil's holes take. Each bounded area outside the art (the inside of
//! an O, any hole of a shape) is an island of the sheet. A bridge is a straight strip of a set
//! width cut out of the art, from an island across the art to the sheet beyond it, or to another
//! island. The bridges are picked as a spanning tree over the islands and the sheet (shortest
//! crossings first), so every island that has room for one is joined to the sheet, then each
//! island gets more bridges, spread around it, up to the number asked for.
//!
//! Crossings are measured along straight lines in one direction at a time (vertical, horizontal,
//! or every 15 degrees for the shortest crossing), spaced half a bridge width apart.

#[cfg(test)]
mod tests;

use std::cell::Cell;

use craft_geom::{FillRule, PathData, SubPath};
use kurbo::{BezPath, ParamCurve, PathEl, Point};

use crate::{BoolOp, DEFAULT_PRECISION, PathOpsError, try_boolean, try_unite_all};

/// The narrowest and widest bridge, in points.
pub const BRIDGE_WIDTH_RANGE: (f64, f64) = (0.01, 1000.0);
/// The most bridges one island is given.
pub const MAX_BRIDGES_PER_ISLAND: usize = 16;
/// The most straight segments the art is followed by.
pub const MAX_STENCIL_SEGMENTS: usize = 200_000;
/// The most outlines the art may have.
pub const MAX_STENCIL_CONTOURS: usize = 4000;
/// How far from the origin the art and its handles may reach, in points: beyond the coordinates a
/// document holds.
pub const MAX_STENCIL_EXTENT: f64 = 1.0e7;
/// The most segments all lines across the art may look at together.
const MAX_WORK: usize = 300_000_000;
/// The most edges the outlines are tested against when finding which ones bound islands.
const MAX_NESTING_WORK: usize = 50_000_000;
/// The most bands one direction's segments are listed in.
const MAX_LINES: f64 = 1000.0;
/// The most lines all islands are looked across along, in every direction together.
const MAX_ISLAND_LINES: usize = 400_000;
/// The most lines on each side of an island's middle, in one direction.
const MAX_LINES_PER_SIDE: usize = 64;
/// The most times segments are listed against the lines they reach, in one direction.
const MAX_BIN_ENTRIES: usize = 20_000_000;
/// The lines across one bridge its width is checked along.
const CHECKS: usize = 5;
/// The directions the shortest crossing is looked for in, in degrees.
const SHORTEST_STEP_DEG: usize = 15;

/// Which way bridges run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeDirection {
    Vertical,
    Horizontal,
    /// The shortest crossing in any of 12 directions, 15 degrees apart.
    Shortest,
}

/// What [`stencil_bridges`] makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BridgeSpec {
    /// Each bridge's width (pt), in [`BRIDGE_WIDTH_RANGE`].
    pub width: f64,
    /// The bridges each island is given, 1 to [`MAX_BRIDGES_PER_ISLAND`] (fewer where it has no
    /// room for more, and more where other islands are joined to the sheet through it).
    pub count: usize,
    pub direction: BridgeDirection,
}

/// The art with its bridges cut out.
#[derive(Clone, Debug, PartialEq)]
pub struct Bridged {
    /// The art as one area, less the bridges.
    pub path: PathData,
    /// The islands the art held.
    pub islands: usize,
    /// The bridges cut.
    pub bridges: usize,
    /// The islands the cut result still encloses: no bridge of the width fits, or the bridges that
    /// fit join them only to each other.
    pub unbridged: usize,
}

/// `art` (each path with its fill rule; open subpaths are closed as a fill closes them) as one
/// area with bridges cut out of it as `spec` says. With no islands, the area is returned as it is
/// and no bridge is cut.
pub fn stencil_bridges(art: &[(&PathData, FillRule)], spec: BridgeSpec) -> Result<Bridged, PathOpsError> {
    let (lo, hi) = BRIDGE_WIDTH_RANGE;
    if !(spec.width.is_finite() && (lo..=hi).contains(&spec.width)) || !(1..=MAX_BRIDGES_PER_ISLAND).contains(&spec.count) {
        return Err(PathOpsError::Degenerate);
    }
    if too_large(art) {
        return Err(PathOpsError::TooComplex);
    }
    let closed: Vec<PathData> =
        art.iter().map(|(p, _)| PathData::new(p.subpaths.iter().map(|sp| SubPath { closed: true, ..sp.clone() }).collect())).collect();
    let refs: Vec<(&PathData, FillRule)> = closed.iter().zip(art).map(|(p, (_, r))| (p, *r)).collect();
    bridge_region(try_unite_all(&refs)?, spec)
}

/// Whether `art` has more than [`MAX_STENCIL_SEGMENTS`] anchors or reaches past
/// [`MAX_STENCIL_EXTENT`]: art refused before it is united.
fn too_large(art: &[(&PathData, FillRule)]) -> bool {
    let anchors: usize = art.iter().flat_map(|(p, _)| &p.subpaths).map(|sp| sp.anchors.len()).sum();
    anchors > MAX_STENCIL_SEGMENTS || !art.iter().all(|(p, _)| within_extent(&p.to_bezpath()))
}

/// [`stencil_bridges`] on `region`, an area with no overlaps.
fn bridge_region(region: PathData, spec: BridgeSpec) -> Result<Bridged, PathOpsError> {
    let contours = contours(&region, spec.width)?;
    let holes = contours.iter().filter(|c| c.hole).count();
    if holes == 0 {
        return Ok(Bridged { path: region, islands: 0, bridges: 0, unbridged: 0 });
    }
    let (chosen, unjoined) = choose(&contours, spec, MAX_WORK)?;
    if chosen.is_empty() {
        return Ok(Bridged { path: region, islands: holes, bridges: 0, unbridged: unjoined });
    }
    let strips = PathData::new(chosen.iter().map(|c| SubPath::polyline(&c.strip, true)).collect());
    let path = try_boolean(&region, FillRule::NonZero, &strips, FillRule::NonZero, BoolOp::Difference, DEFAULT_PRECISION)?;
    // The islands the result still encloses, counted on the cut outlines themselves (removing art
    // never encloses a new one).
    let unbridged = match self::contours(&path, spec.width) {
        Ok(left) => left.iter().filter(|c| c.hole).count(),
        Err(_) => unjoined,
    };
    Ok(Bridged { path, islands: holes, bridges: chosen.len(), unbridged })
}

/// The bridges `spec` asks for across the art that `contours` outline, and how many islands they
/// leave apart from the sheet. Lines across the art look at no more than `work` segments in all;
/// past that, the art is refused when the islands are not all joined yet, and otherwise the extra
/// bridges found so far are kept.
fn choose(contours: &[Contour], spec: BridgeSpec, work: usize) -> Result<(Vec<Bridge>, usize), PathOpsError> {
    let degrees: Vec<f64> = match spec.direction {
        BridgeDirection::Vertical => vec![90.0],
        BridgeDirection::Horizontal => vec![0.0],
        BridgeDirection::Shortest => (0..180).step_by(SHORTEST_STEP_DEG).map(|d| d as f64).collect(),
    };
    let budget = work / degrees.len().max(1);
    let mut views = vec![];
    for deg in degrees {
        views.push(View::new(contours, deg.to_radians(), spec.width, budget)?);
    }
    let mut picker = Picker::new(contours, &views, spec.width);
    picker.connect();
    if views.iter().any(View::exhausted) && picker.unjoined() > 0 {
        return Err(PathOpsError::TooComplex);
    }
    picker.spread(spec.count);
    let unjoined = picker.unjoined();
    Ok((picker.chosen, unjoined))
}

/// One closed outline of the art, followed in straight segments.
struct Contour {
    points: Vec<Point>,
    /// Whether it bounds an island (it lies inside an odd number of the others).
    hole: bool,
    /// Its area, without sign.
    area: f64,
    /// Its box: (min x, min y, max x, max y).
    bounds: (f64, f64, f64, f64),
}

/// The outlines of `region` (an area with no overlaps) followed within a tolerance set by the
/// bridge width: a twentieth of it, from 0.002 to 0.25 pt, raised for a curve that would need more
/// than [`craft_geom::MAX_FLATTEN_PER_CURVE`] segments. Art reaching past
/// [`MAX_STENCIL_EXTENT`] is refused.
fn contours(region: &PathData, width: f64) -> Result<Vec<Contour>, PathOpsError> {
    if !within_extent(&region.to_bezpath()) {
        return Err(PathOpsError::TooComplex);
    }
    let tolerance = (width / 20.0).clamp(0.002, 0.25);
    let mut out: Vec<Contour> = vec![];
    let mut segments = 0usize;
    for sp in &region.subpaths {
        let room = MAX_STENCIL_SEGMENTS.saturating_sub(segments);
        let mut bp = BezPath::new();
        sp.to_bezpath_into(&mut bp);
        let mut points = follow(&bp, tolerance, room)?;
        while points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        if points.len() < 3 {
            continue;
        }
        segments += points.len();
        if segments > MAX_STENCIL_SEGMENTS || out.len() >= MAX_STENCIL_CONTOURS {
            return Err(PathOpsError::TooComplex);
        }
        if points.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
            return Err(PathOpsError::NonFinite);
        }
        let area = signed_area(&points).abs();
        let bounds = points.iter().fold((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY), |b, p| {
            (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y))
        });
        out.push(Contour { points, hole: false, area, bounds });
    }
    // An outline too small to see is dropped, so its inside counts as art (or, for a speck of art,
    // as the area around it).
    let tiny = (tolerance * tolerance).max(1e-9);
    out.retain(|c| c.area > tiny);
    // An outline inside an odd number of others bounds an island. Each outline is placed by the
    // middle of its longest edge: outlines of an area with no overlaps meet at most at corners, so
    // that point lies on no other outline.
    let mut work = 0usize;
    let mut depths = Vec::with_capacity(out.len());
    for (i, c) in out.iter().enumerate() {
        let p = longest_edge_middle(&c.points);
        let mut depth = 0usize;
        for (j, o) in out.iter().enumerate() {
            if j == i || o.area < c.area || !holds_box(o.bounds, c.bounds) {
                continue;
            }
            work += o.points.len();
            if work > MAX_NESTING_WORK {
                return Err(PathOpsError::TooComplex);
            }
            if contains(o, p) {
                depth += 1;
            }
        }
        depths.push(depth);
    }
    for (c, depth) in out.iter_mut().zip(depths) {
        c.hole = depth % 2 == 1;
    }
    Ok(out)
}

/// The middle of the longest edge of the closed outline through `points`.
fn longest_edge_middle(points: &[Point]) -> Point {
    let n = points.len();
    (0..n)
        .map(|i| (points[i], points[(i + 1) % n]))
        .max_by(|(a, b), (c, d)| a.distance(*b).total_cmp(&c.distance(*d)))
        .map_or(Point::ZERO, |(a, b)| a.midpoint(b))
}

/// Whether box `outer` holds box `inner`, edges included. Boxes are (min x, min y, max x, max y).
fn holds_box(outer: (f64, f64, f64, f64), inner: (f64, f64, f64, f64)) -> bool {
    outer.0 <= inner.0 && outer.1 <= inner.1 && outer.2 >= inner.2 && outer.3 >= inner.3
}

/// Whether `bp` and its handles stay within [`MAX_STENCIL_EXTENT`] of the origin.
fn within_extent(bp: &BezPath) -> bool {
    let reach = bp.control_box();
    [reach.x0, reach.y0, reach.x1, reach.y1].iter().all(|v| v.abs() <= MAX_STENCIL_EXTENT)
}

/// The points that follow outline `bp` within `tolerance`, segment by segment, or
/// [`PathOpsError::TooComplex`] once they pass `room` (and one more for a closed outline's
/// repeated start, which the caller drops). It stops at the first segment past the limit.
fn follow(bp: &BezPath, tolerance: f64, room: usize) -> Result<Vec<Point>, PathOpsError> {
    let limit = room.saturating_add(1);
    let mut points: Vec<Point> = vec![];
    for seg in bp.segments() {
        if points.is_empty() {
            points.push(seg.start());
        }
        let mut over = false;
        craft_geom::flatten([PathEl::MoveTo(seg.start()), seg.as_path_el()], tolerance, |el| match el {
            PathEl::LineTo(p) if points.len() < limit => points.push(p),
            PathEl::LineTo(_) => over = true,
            _ => {}
        });
        if over {
            return Err(PathOpsError::TooComplex);
        }
    }
    Ok(points)
}

fn signed_area(points: &[Point]) -> f64 {
    let n = points.len();
    (0..n).map(|i| points[i].to_vec2().cross(points[(i + 1) % n].to_vec2())).sum::<f64>() / 2.0
}

/// Whether `p` is inside contour `c` (even-odd).
fn contains(c: &Contour, p: Point) -> bool {
    let (x0, y0, x1, y1) = c.bounds;
    if p.x < x0 || p.x > x1 || p.y < y0 || p.y > y1 {
        return false;
    }
    let n = c.points.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (c.points[i], c.points[(i + 1) % n]);
        if (a.y <= p.y) != (b.y <= p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
            inside = !inside;
        }
    }
    inside
}

/// The area outside the art that a point is in: 0 for the sheet around the art, `i + 1` for the
/// island contour `i` bounds.
type Area = usize;
const SHEET: Area = 0;

/// The art seen along lines in one direction: rotated so the lines run along x, with its segments
/// listed by the bands of y they reach.
struct View {
    angle: f64,
    /// Rotated segments, each with the contour it belongs to.
    segments: Vec<(Point, Point, usize)>,
    /// The lowest y of the rotated art.
    y0: f64,
    /// The band height, and the lines' spacing.
    step: f64,
    bands: Vec<Vec<u32>>,
    /// Each contour's lowest and highest rotated y.
    ranges: Vec<(f64, f64)>,
    /// The segments lines may still look at; past it, every line is missing.
    budget: Cell<usize>,
}

/// Where a line crosses the art: the edges in order (even-odd pairs are the art), and the area
/// each stretch outside the art is in (one more than there are pairs).
struct Line {
    xs: Vec<f64>,
    areas: Vec<Area>,
}

impl View {
    fn new(contours: &[Contour], angle: f64, width: f64, budget: usize) -> Result<Self, PathOpsError> {
        let (sin, cos) = angle.sin_cos();
        let rotate = |p: Point| Point::new(p.x * cos + p.y * sin, -p.x * sin + p.y * cos);
        let mut segments = vec![];
        let mut ranges = vec![];
        let (mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY);
        for (k, c) in contours.iter().enumerate() {
            let pts: Vec<Point> = c.points.iter().map(|p| rotate(*p)).collect();
            let (lo, hi) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
            ranges.push((lo, hi));
            y0 = y0.min(lo);
            y1 = y1.max(hi);
            for i in 0..pts.len() {
                segments.push((pts[i], pts[(i + 1) % pts.len()], k));
            }
        }
        let span = (y1 - y0).max(width);
        let step = (width / 2.0).max(span / MAX_LINES);
        let count = (span / step).ceil() as usize + 1;
        let mut bands: Vec<Vec<u32>> = vec![vec![]; count];
        let band = |y: f64| (((y - y0) / step).floor().max(0.0) as usize).min(count - 1);
        let mut entries = 0usize;
        for (i, (a, b, _)) in segments.iter().enumerate() {
            let (from, to) = (band(a.y.min(b.y)), band(a.y.max(b.y)));
            entries += to - from + 1;
            if entries > MAX_BIN_ENTRIES {
                return Err(PathOpsError::TooComplex);
            }
            for list in &mut bands[from..=to] {
                list.push(i as u32);
            }
        }
        Ok(View { angle, segments, y0, step, bands, ranges, budget: Cell::new(budget) })
    }

    /// Whether lines looked at more segments than the budget allows.
    fn exhausted(&self) -> bool {
        self.budget.get() == 0
    }

    /// The art along the line at rotated y `y`, `None` where its edges don't pair up or the budget
    /// is spent.
    fn line(&self, y: f64, contours: &[Contour]) -> Option<Line> {
        let band = ((y - self.y0) / self.step).floor();
        let list = self.bands.get(if band < 0.0 { usize::MAX } else { band as usize })?;
        let left = self.budget.get().checked_sub(list.len()).filter(|left| *left > 0);
        self.budget.set(left.unwrap_or(0));
        left?;
        let mut hits: Vec<(f64, usize)> = vec![];
        for &i in list {
            let (a, b, k) = self.segments[i as usize];
            if (a.y <= y) != (b.y <= y) {
                hits.push((a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y), k));
            }
        }
        if hits.len() % 2 == 1 {
            return None;
        }
        hits.sort_by(|p, q| p.0.total_cmp(&q.0));
        // Walking along the line, each contour crossed an odd number of times so far holds the
        // point; the smallest island contour holding a stretch outside the art is its island.
        let mut open: Vec<usize> = vec![];
        let mut areas = vec![SHEET];
        for (n, (_, k)) in hits.iter().enumerate() {
            match open.iter().position(|o| o == k) {
                Some(at) => {
                    open.swap_remove(at);
                }
                None => open.push(*k),
            }
            if n % 2 == 1 {
                let island = open.iter().filter(|o| contours[**o].hole).min_by(|a, b| contours[**a].area.total_cmp(&contours[**b].area));
                areas.push(island.map_or(SHEET, |k| k + 1));
            }
        }
        Some(Line { xs: hits.into_iter().map(|(x, _)| x).collect(), areas })
    }

    /// The point at rotated `(x, y)` in the art's own coordinates.
    fn unrotate(&self, x: f64, y: f64) -> Point {
        let (sin, cos) = self.angle.sin_cos();
        Point::new(x * cos - y * sin, x * sin + y * cos)
    }
}

/// A crossing of the art along one line, between the areas on either side of it.
#[derive(Clone, Copy, Debug)]
struct Crossing {
    view: usize,
    y: f64,
    /// Where it enters and leaves the art.
    from: f64,
    to: f64,
    areas: (Area, Area),
    /// How far its line is from the middle of the island it starts from.
    off_middle: f64,
}

impl Crossing {
    fn len(&self) -> f64 {
        self.to - self.from
    }
}

/// A bridge that fits: its crossing, its middle and its strip's corners.
struct Bridge {
    areas: (Area, Area),
    middle: Point,
    strip: Vec<Point>,
}

struct Picker<'a> {
    contours: &'a [Contour],
    views: &'a [View],
    width: f64,
    crossings: Vec<Crossing>,
    /// The crossings that touch each area.
    touching: Vec<Vec<usize>>,
    parent: Vec<usize>,
    chosen: Vec<Bridge>,
}

impl<'a> Picker<'a> {
    fn new(contours: &'a [Contour], views: &'a [View], width: f64) -> Self {
        let areas = contours.len() + 1;
        let holes = contours.iter().filter(|c| c.hole).count().max(1);
        // Each island is looked across along lines through its middle and out from it on both
        // sides, half a width apart, or fewer where there are many islands.
        let each_side = (MAX_ISLAND_LINES / (holes * views.len()).max(1)).clamp(1, MAX_LINES_PER_SIDE) as f64;
        let mut crossings = vec![];
        for (v, view) in views.iter().enumerate() {
            for (k, _) in contours.iter().enumerate().filter(|(_, c)| c.hole) {
                let island = k + 1;
                let (lo, hi) = view.ranges[k];
                let (middle, reach) = ((lo + hi) / 2.0, (hi - lo) / 2.0);
                let step = (width / 2.0).max(reach / each_side);
                let mut ys = vec![middle];
                let mut d = step;
                while d < reach {
                    ys.extend([middle - d, middle + d]);
                    d += step;
                }
                for y in ys {
                    let Some(line) = view.line(y, contours) else { continue };
                    for (j, pair) in line.xs.as_chunks::<2>().0.iter().enumerate() {
                        let (a, b) = (line.areas[j], line.areas[j + 1]);
                        if a == b || (a != island && b != island) {
                            continue;
                        }
                        crossings.push(Crossing { view: v, y, from: pair[0], to: pair[1], areas: (a, b), off_middle: (y - middle).abs() });
                    }
                }
            }
        }
        // Shortest first; among crossings of about the same length, the one nearest the middle of
        // its island.
        let grain = width * 0.05;
        crossings.sort_by(|p, q| (p.len() / grain).round().total_cmp(&(q.len() / grain).round()).then(p.off_middle.total_cmp(&q.off_middle)));
        let mut touching = vec![vec![]; areas];
        for (i, c) in crossings.iter().enumerate() {
            touching[c.areas.0].push(i);
            touching[c.areas.1].push(i);
        }
        Picker { contours, views, width, crossings, touching, parent: (0..areas).collect(), chosen: vec![] }
    }

    fn root(&mut self, mut a: usize) -> usize {
        while self.parent[a] != a {
            self.parent[a] = self.parent[self.parent[a]];
            a = self.parent[a];
        }
        a
    }

    /// Join every island to the sheet by the shortest crossings that fit, as a spanning tree. Bridges
    /// that only join islands that stay apart from the sheet are not cut.
    fn connect(&mut self) {
        let mut apart = self.contours.iter().filter(|c| c.hole).count();
        for i in 0..self.crossings.len() {
            if apart == 0 {
                break;
            }
            let c = self.crossings[i];
            let (a, b) = (self.root(c.areas.0), self.root(c.areas.1));
            if a == b {
                continue;
            }
            if let Some(bridge) = self.fit(&c) {
                self.parent[a] = b;
                self.chosen.push(bridge);
                apart -= 1;
            }
        }
        let sheet = self.root(SHEET);
        for bridge in std::mem::take(&mut self.chosen) {
            if self.root(bridge.areas.0) == sheet {
                self.chosen.push(bridge);
            }
        }
    }

    /// Give each island more bridges, up to `count`, each far from the island's others (at least
    /// two widths away, and as short as can be among those at least 80% as far as the farthest),
    /// and no more than twice as long as its shortest crossing that a bridge fits plus a width.
    /// A crossing a bridge does not fit is not tried again, and once the lines' budget is spent no
    /// more bridges are found.
    fn spread(&mut self, count: usize) {
        let sheet = self.root(SHEET);
        let mut misfit = vec![false; self.crossings.len()];
        for island in 1..self.touching.len() {
            if !self.contours[island - 1].hole || self.root(island) != sheet {
                continue;
            }
            let mut limit = None;
            loop {
                let mine: Vec<Point> = self.chosen.iter().filter(|b| b.areas.0 == island || b.areas.1 == island).map(|b| b.middle).collect();
                if mine.len() >= count {
                    break;
                }
                let longest = match limit {
                    Some(l) => l,
                    None => {
                        // The island's crossings are listed shortest first, so the first that fits
                        // is its shortest bridge.
                        let mut shortest = None;
                        for &i in &self.touching[island] {
                            if misfit[i] {
                                continue;
                            }
                            if self.fit(&self.crossings[i]).is_some() {
                                shortest = Some(self.crossings[i].len());
                                break;
                            }
                            misfit[i] = true;
                        }
                        let Some(shortest) = shortest else { break };
                        *limit.insert(shortest * 2.0 + self.width)
                    }
                };
                let mut ranked: Vec<(f64, usize)> = self.touching[island]
                    .iter()
                    .filter(|&&i| !misfit[i] && self.crossings[i].len() <= longest)
                    .map(|&i| {
                        let m = self.middle(&self.crossings[i]);
                        (mine.iter().map(|p| p.distance(m)).fold(f64::INFINITY, f64::min), i)
                    })
                    .filter(|(d, _)| *d >= self.width * 2.0)
                    .collect();
                // The crossings nearly as far as the farthest go first, in the order `connect`
                // takes them (shortest, then nearest the middle); then the rest, farthest first.
                let far = ranked.iter().map(|r| r.0).fold(0.0, f64::max) * 0.8;
                ranked.sort_by(|p, q| (q.0 >= far).cmp(&(p.0 >= far)).then_with(|| if p.0 >= far { p.1.cmp(&q.1) } else { q.0.total_cmp(&p.0) }));
                let mut found = None;
                for &(_, i) in &ranked {
                    match self.fit(&self.crossings[i]) {
                        Some(bridge) => {
                            found = Some(bridge);
                            break;
                        }
                        None => misfit[i] = true,
                    }
                }
                match found {
                    Some(bridge) => self.chosen.push(bridge),
                    None => break,
                }
                if self.views.iter().any(View::exhausted) {
                    return;
                }
            }
        }
    }

    /// The islands not joined to the sheet.
    fn unjoined(&mut self) -> usize {
        let sheet = self.root(SHEET);
        (1..self.parent.len()).filter(|&a| self.contours[a - 1].hole).filter(|&a| self.root(a) != sheet).count()
    }

    fn middle(&self, c: &Crossing) -> Point {
        self.views[c.view].unrotate((c.from + c.to) / 2.0, c.y)
    }

    /// The bridge of crossing `c`, if a strip of the width fits there: along lines across its
    /// width, the art is crossed once between the same two areas, and the strip ends outside the
    /// art on each of them.
    fn fit(&self, c: &Crossing) -> Option<Bridge> {
        let view = &self.views[c.view];
        let half = self.width / 2.0;
        let (mut start, mut end) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut room_before, mut room_after) = (f64::NEG_INFINITY, f64::INFINITY);
        for k in 0..CHECKS {
            let y = c.y - half + self.width * k as f64 / (CHECKS - 1) as f64;
            let line = view.line(y, self.contours)?;
            let last = line.areas.len() - 1;
            // The crossing between the same two areas that overlaps this one, the nearest if several do.
            let (j, _) = line
                .xs
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .filter(|(j, p)| (line.areas[*j], line.areas[j + 1]) == c.areas && p[1] > c.from - self.width && p[0] < c.to + self.width)
                .min_by(|(_, p), (_, q)| (p[0] - c.from).abs().total_cmp(&(q[0] - c.from).abs()))?;
            start = start.min(line.xs[2 * j]);
            end = end.max(line.xs[2 * j + 1]);
            if j > 0 {
                room_before = room_before.max(line.xs[2 * j - 1]);
            }
            if j + 1 < last {
                room_after = room_after.min(line.xs[2 * j + 2]);
            }
        }
        if room_before >= start || room_after <= end {
            return None;
        }
        // The strip reaches a little past the art at each end, so no sliver of it is left.
        let reach = self.width.min(1.0);
        let s0 = (start - reach).max((room_before + start) / 2.0);
        let s1 = (end + reach).min((room_after + end) / 2.0);
        let strip = [(s0, c.y - half), (s1, c.y - half), (s1, c.y + half), (s0, c.y + half)].map(|(x, y)| view.unrotate(x, y)).to_vec();
        Some(Bridge { areas: c.areas, middle: self.middle(c), strip })
    }
}
