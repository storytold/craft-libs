//! Auto Grouping: which traced path lies inside which, so that a result can be grouped the way it
//! is nested (a ring with what fills its hole, a face with its eyes).

use crate::TracedPath;
use kurbo::{PathEl, Point, Rect};

/// How far (px) outside an outline a point may lie and still count as inside it: two paths that
/// share an edge are each fitted on their own.
const NEAR: f64 = 0.5;

/// A path's outer contour as a polygon, and its box.
struct Outer {
    pts: Vec<Point>,
    bbox: Rect,
    /// A closed, filled contour: something can lie inside it.
    holds: bool,
    /// Where something can: for each pixel row from `bbox.y0 - NEAR` down, the edges (each from
    /// the point of that index to the next) that pass through it or within [`NEAR`] of it. A
    /// point is tested against the edges of its row, not all of a long outline's.
    /// Empty past the shared row budget: then [`inside_or_near`] tests `every` edge instead.
    rows: Vec<Vec<u32>>,
    every: Vec<u32>,
}

/// The most row vectors all contours together may hold (about 44 bytes each): tall contours of a
/// large image would otherwise hold gigabytes of row headers at once. The test cap is small so a
/// test image can cross it.
const MAX_ROWS: usize = if cfg!(test) { 1 << 12 } else { 8 << 20 };

fn outer(tp: &TracedPath, budget: &mut usize) -> Option<Outer> {
    let (mut pts, mut subpaths, mut closed) = (vec![], 0, false);
    kurbo::flatten(tp.path.to_bezpath(), 0.2, |el| match el {
        PathEl::MoveTo(p) => {
            subpaths += 1;
            if subpaths == 1 {
                pts.push(p);
            }
        }
        PathEl::LineTo(p) if subpaths == 1 => pts.push(p),
        PathEl::ClosePath if subpaths == 1 => closed = true,
        _ => {}
    });
    let first = *pts.first()?;
    let bbox = pts.iter().fold(Rect::from_points(first, first), |b, p| b.union_pt(*p));
    if !bbox.is_finite() {
        return None;
    }
    let holds = closed && tp.stroke.is_none();
    // (Traced paths lie in the image: a box taller than any image is nothing to hold things.)
    let holds = holds && bbox.height() < 1e6 && u32::try_from(pts.len()).is_ok();
    let (mut rows, mut every) = (vec![], vec![]);
    let wanted = (bbox.height() + 2.0 * NEAR) as usize + 1;
    if holds && wanted <= *budget {
        *budget -= wanted;
        rows = vec![vec![]; wanted];
        for (i, (a, b)) in pts.iter().zip(pts.iter().cycle().skip(1)).enumerate() {
            let (top, bottom) = (a.y.min(b.y) - bbox.y0, a.y.max(b.y) - bbox.y0 + 2.0 * NEAR);
            for row in rows.iter_mut().take(bottom as usize + 1).skip(top.max(0.0) as usize) {
                row.push(i as u32);
            }
        }
    } else if holds {
        every = (0..pts.len() as u32).collect();
    }
    Some(Outer { pts, bbox, holds, rows, every })
}

/// Whether `p` lies inside the closed outline of `o`, or within [`NEAR`] of its edge.
fn inside_or_near(p: Point, o: &Outer) -> bool {
    let row = p.y - (o.bbox.y0 - NEAR);
    let edges: &[u32] = if o.rows.is_empty() && !o.every.is_empty() {
        &o.every
    } else {
        match (row >= 0.0).then(|| o.rows.get(row as usize)).flatten() {
            Some(edges) => edges,
            None => return false,
        }
    };
    let mut inside = false;
    for &i in edges {
        let (Some(a), Some(b)) = (o.pts.get(i as usize), o.pts.get((i as usize + 1) % o.pts.len().max(1))) else { continue };
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
        let d = *b - *a;
        let t = if d.hypot2() > 0.0 { ((p - *a).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
        if (p - (*a + d * t)).hypot2() <= NEAR * NEAR {
            return true;
        }
    }
    inside
}

/// The most paths grouped.
const MAX_GROUPED_PATHS: usize = 20_000;

/// For each of `paths` (bottom-most first), the path it lies inside, if any: the smallest filled
/// path whose outer contour holds all of its own. The path that covers the whole `w` × `h` image
/// (the paper) is nobody's parent: everything would be inside it.
///
/// Drawing each path together with those inside it (in their own order) paints the picture
/// `paths` paints in its own order: where the paths can overlap (`stacked`), a path only gets a parent below it, and only
/// if no path that would then come after it instead of before it shares any of its box.
pub(crate) fn parents(paths: &[TracedPath], w: usize, h: usize, stacked: bool) -> Vec<Option<usize>> {
    // Deeply nested paths are still tested against each other (quadratic): past this many, they stay ungrouped.
    if paths.len() > MAX_GROUPED_PATHS {
        return vec![None; paths.len()];
    }
    let mut budget = MAX_ROWS;
    let outers: Vec<Option<Outer>> = paths.iter().map(|tp| outer(tp, &mut budget)).collect();
    let paper = Rect::new(1.0, 1.0, w as f64 - 1.0, h as f64 - 1.0);
    let mut parent: Vec<Option<usize>> = vec![None; paths.len()];
    // How many paths each is inside of.
    let mut depth = vec![0usize; paths.len()];
    // The paths something can lie inside, by the area of their boxes.
    let mut by_area: Vec<usize> = (0..paths.len()).filter(|&p| outers.get(p).and_then(Option::as_ref).is_some_and(|o| o.holds)).collect();
    let area = |p: &usize| outers.get(*p).and_then(Option::as_ref).map_or(0.0, |o| o.bbox.area());
    by_area.sort_by(|a, b| area(a).total_cmp(&area(b)));
    // The paths something can lie inside, smallest first, with their boxes and NEAR around them.
    // A box that holds another holds its center. The paper holds nothing.
    let holder_boxes: Vec<(usize, Rect)> = by_area
        .iter()
        .filter_map(|&p| {
            let o = outers.get(p)?.as_ref()?.bbox;
            let covers = o.x0 <= paper.x0 && o.y0 <= paper.y0 && o.x1 >= paper.x1 && o.y1 >= paper.y1;
            (!covers).then(|| (p, o.inflate(NEAR, NEAR)))
        })
        .collect();
    // Where paths can overlap, every path with its box, bottom-most first.
    let below_boxes: Vec<(usize, Rect)> =
        if stacked { outers.iter().enumerate().filter_map(|(y, oy)| Some((y, oy.as_ref()?.bbox))).collect() } else { vec![] };
    let grid = Grid::new(w, h, holder_boxes.iter().chain(&below_boxes).map(|&(_, r)| r));
    // For each cell, the paths in those lists whose box reaches it, in the same order.
    let holders_at = grid.lists(&holder_boxes);
    let below_at = grid.lists(&below_boxes);
    for (x, ox) in outers.iter().enumerate() {
        let Some(ox) = ox else { continue };
        // The paths whose box holds its own, smallest first.
        let candidates = holders_at.get(grid.cell(ox.bbox.center())).into_iter().flatten();
        let mut holders = candidates.filter_map(|&p| Some((p, outers.get(p)?.as_ref()?))).filter(|&(p, op)| {
            #[cfg(test)]
            BOX_TESTS.with(|n| n.set(n.get() + 1));
            let (o, i) = (op.bbox, ox.bbox);
            o.x0 - NEAR <= i.x0 && o.y0 - NEAR <= i.y0 && o.x1 + NEAR >= i.x1 && o.y1 + NEAR >= i.y1 && o.area() > i.area() && !(stacked && p > x)
        });
        // The paths under `x` that share some of its box: it may not come to lie under any of them.
        // (Found only once some path holds its first point: most paths are inside none.)
        let under = std::cell::OnceCell::new();
        let under = || {
            under.get_or_init(|| {
                let overlap = |y: usize| {
                    #[cfg(test)]
                    BOX_TESTS.with(|n| n.set(n.get() + 1));
                    outers.get(y).and_then(Option::as_ref).map(|oy| oy.bbox.intersect(ox.bbox)).filter(|r| r.area() > 0.0)
                };
                // The paths below `x` in each cell its box reaches, unless that is more paths to
                // look at than there are below it.
                let mut lists = vec![];
                let mut count = 0;
                let few = grid.reach(ox.bbox) <= x
                    && grid.cells(ox.bbox).all(|c| {
                        let list = below_at.get(c).map_or(&[][..], |list| list.get(..list.partition_point(|&y| y < x)).unwrap_or_default());
                        count += list.len();
                        lists.push((c, list));
                        count <= x
                    });
                if !few {
                    return (0..x).filter(|&y| overlap(y).is_some()).collect::<Vec<_>>();
                }
                // A path in several of the cells is taken from the one its overlap with `x` starts in.
                let from = |c: usize, y: usize| overlap(y).is_some_and(|r| grid.cell(r.origin()) == c);
                lists.into_iter().flat_map(|(c, list)| list.iter().copied().filter(move |&y| from(c, y))).collect()
            })
        };
        let found = holders.find(|&(p, op)| {
            ox.pts.first().is_some_and(|pt| inside_or_near(*pt, op))
                && (!stacked || under().iter().all(|&y| !comes_after(&parent, &depth, y, p)))
                && ox.pts.iter().all(|pt| inside_or_near(*pt, op))
        });
        if let (Some((p, _)), Some(slot)) = (found, parent.get_mut(x)) {
            *slot = Some(p);
            let deep = depth.get(p).copied().unwrap_or(0) + 1;
            if let Some(d) = depth.get_mut(x) {
                *d = deep;
            }
        }
    }
    parent
}

/// The image cut into square cells, to find the paths whose boxes reach a place without comparing
/// every pair of paths. Places outside the image belong to its edge cells.
struct Grid {
    size: f64,
    cols: usize,
    rows: usize,
}

impl Grid {
    /// How many cells each box may be listed in on average.
    const BUDGET: usize = 32;

    /// About one cell for each of `boxes`, with larger cells where the boxes would otherwise be
    /// listed in more than [`Grid::BUDGET`] cells each on average.
    fn new(w: usize, h: usize, boxes: impl Iterator<Item = Rect> + Clone) -> Self {
        let n = boxes.clone().count();
        let mut size = (w as f64 * h as f64 / n.clamp(1, 1 << 20) as f64).sqrt().max(1.0);
        loop {
            let count = |len: usize| ((len as f64 / size).ceil() as usize).max(1);
            let grid = Self { size, cols: count(w), rows: count(h) };
            let listed = boxes.clone().fold(0usize, |sum, r| sum.saturating_add(grid.reach(r)));
            if listed <= Self::BUDGET.saturating_mul(n) || (grid.cols == 1 && grid.rows == 1) {
                return grid;
            }
            size *= 2.0;
        }
    }

    /// How many cells `r` reaches.
    fn reach(&self, r: Rect) -> usize {
        let span = |a: f64, b: f64, count: usize| self.index(b, count).saturating_sub(self.index(a, count)) + 1;
        span(r.x0, r.x1, self.cols).saturating_mul(span(r.y0, r.y1, self.rows))
    }

    /// The column (or row, of `count`) that `v` falls in.
    fn index(&self, v: f64, count: usize) -> usize {
        (v / self.size).floor().clamp(0.0, count.saturating_sub(1) as f64) as usize
    }

    fn cell(&self, p: Point) -> usize {
        self.index(p.y, self.rows) * self.cols + self.index(p.x, self.cols)
    }

    /// The cells `r` reaches.
    fn cells(&self, r: Rect) -> impl Iterator<Item = usize> {
        let (cols, (c0, c1)) = (self.cols, (self.index(r.x0, self.cols), self.index(r.x1, self.cols)));
        (self.index(r.y0, self.rows)..=self.index(r.y1, self.rows)).flat_map(move |row| (c0..=c1).map(move |col| row * cols + col))
    }

    /// For each cell, the `items` whose box reaches it, in the order they come in.
    fn lists(&self, items: &[(usize, Rect)]) -> Vec<Vec<usize>> {
        let mut lists = vec![vec![]; self.cols * self.rows];
        for &(i, r) in items {
            for c in self.cells(r) {
                if let Some(list) = lists.get_mut(c) {
                    list.push(i);
                }
            }
        }
        lists
    }
}

/// Whether `y` is drawn after `p` and all that is inside `p`: it is in another branch of the
/// nesting, and that branch starts above the one `p` is in.
fn comes_after(parent: &[Option<usize>], depth: &[usize], y: usize, p: usize) -> bool {
    let up = |i: usize| parent.get(i).copied().flatten();
    let deep = |i: usize| depth.get(i).copied().unwrap_or(0);
    let (mut a, mut b) = (y, p);
    while deep(a) > deep(b) {
        let Some(q) = up(a) else { break };
        a = q;
    }
    while deep(b) > deep(a) {
        let Some(q) = up(b) else { break };
        b = q;
    }
    // (Equal: one is the other, or inside it.)
    while a != b && up(a) != up(b) {
        let (Some(qa), Some(qb)) = (up(a), up(b)) else { break };
        (a, b) = (qa, qb);
    }
    a > b
}

#[cfg(test)]
thread_local! {
    /// How many times [`parents`] has compared two paths' boxes on this thread.
    static BOX_TESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_geom::PathData;

    fn disc(c: (f64, f64), r: f64, color: u8) -> TracedPath {
        let path = PathData::from_bezpath(&kurbo::Shape::to_path(&kurbo::Circle::new(c, r), 0.01));
        TracedPath { path, color: [color; 3], pixels: 1, stroke: None, shape: None, gradient: None, square_caps: false }
    }

    #[test]
    fn a_path_is_inside_the_smallest_path_that_holds_it() {
        // The paper, a disc, a disc inside it, a dot inside that, and a disc beside them.
        let paths = [
            disc((50.0, 50.0), 80.0, 255),
            disc((30.0, 50.0), 20.0, 0),
            disc((30.0, 50.0), 10.0, 100),
            disc((30.0, 50.0), 3.0, 0),
            disc((75.0, 50.0), 10.0, 0),
        ];
        for stacked in [false, true] {
            assert_eq!(parents(&paths, 100, 100, stacked), [None, None, Some(1), Some(2), None]);
        }
    }

    #[test]
    fn stacked_paths_are_not_reordered_where_they_overlap() {
        // A dot over a disc, with a wider disc between them in the stack that covers the dot's
        // place without holding the first disc: grouped with the first disc, the dot would go under it.
        let paths = [disc((30.0, 50.0), 20.0, 0), disc((60.0, 50.0), 24.0, 100), disc((35.0, 50.0), 3.0, 200)];
        assert_eq!(parents(&paths, 100, 100, true), [None, None, None]);
        // Where paths cannot overlap, either order paints the same.
        assert_eq!(parents(&paths, 100, 100, false), [None, None, Some(0)]);
    }

    /// Smooth noise: a value per 3 px lattice point, blended between them.
    fn noise(w: u32, h: u32) -> crate::Raster {
        let at = |x: u32, y: u32| {
            let mut v = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
            v ^= v >> 15;
            v = v.wrapping_mul(0x2C1B_3C6D);
            v ^= v >> 12;
            f64::from(v & 0xFF)
        };
        crate::Raster::from_fn(w, h, |x, y| {
            let (gx, gy) = (x / 3, y / 3);
            let (fx, fy) = (f64::from(x % 3) / 3.0, f64::from(y % 3) / 3.0);
            let top = at(gx, gy) * (1.0 - fx) + at(gx + 1, gy) * fx;
            let bottom = at(gx, gy + 1) * (1.0 - fx) + at(gx + 1, gy + 1) * fx;
            let v = (top * (1.0 - fy) + bottom * fy).round() as u8;
            [v, v, v, 255]
        })
    }

    /// Rings inside rings: three colors in 5 px bands around four centers.
    fn rings() -> crate::Raster {
        const COLORS: [[u8; 4]; 3] = [[20, 20, 20, 255], [200, 40, 40, 255], [250, 250, 250, 255]];
        crate::Raster::from_fn(160, 160, |x, y| {
            let (cx, cy) = (if x < 80 { 40.0 } else { 120.0 }, if y < 80 { 40.0 } else { 120.0 });
            let d = Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5).distance(Point::new(cx, cy));
            COLORS.get((d / 5.0) as usize % 3).copied().unwrap_or_default()
        })
    }

    /// Nine faces on white: a yellow disc with two dark eyes and a dark mouth.
    fn faces() -> crate::Raster {
        crate::Raster::from_fn(150, 150, |x, y| {
            let (px, py) = (f64::from(x % 50) + 0.5, f64::from(y % 50) + 0.5);
            let at = |cx: f64, cy: f64| Point::new(px, py).distance(Point::new(cx, cy));
            if at(18.0, 20.0) <= 3.0 || at(32.0, 20.0) <= 3.0 || ((17.0..33.0).contains(&px) && (31.0..35.0).contains(&py)) {
                [30, 30, 30, 255]
            } else if at(25.0, 25.0) <= 20.0 {
                [240, 200, 40, 255]
            } else {
                [255, 255, 255, 255]
            }
        })
    }

    fn traced(img: &crate::Raster, method: crate::Method) -> Vec<TracedPath> {
        let params = crate::TraceParams {
            mode: crate::Mode::Color,
            palette: crate::Palette::Limited,
            colors: 8,
            paths: 95.0,
            corners: 50.0,
            noise: 1,
            method,
            ..crate::TraceParams::default()
        };
        crate::trace(img, &params).paths
    }

    /// [`parents`] comparing the box of every path with every other's.
    fn parents_by_pairs(paths: &[TracedPath], w: usize, h: usize, stacked: bool) -> Vec<Option<usize>> {
        let mut budget = MAX_ROWS;
        let outers: Vec<Option<Outer>> = paths.iter().map(|tp| outer(tp, &mut budget)).collect();
        let paper = Rect::new(1.0, 1.0, w as f64 - 1.0, h as f64 - 1.0);
        let mut parent: Vec<Option<usize>> = vec![None; paths.len()];
        let mut depth = vec![0usize; paths.len()];
        let mut by_area: Vec<usize> = (0..paths.len()).filter(|&p| outers.get(p).and_then(Option::as_ref).is_some_and(|o| o.holds)).collect();
        let area = |p: &usize| outers.get(*p).and_then(Option::as_ref).map_or(0.0, |o| o.bbox.area());
        by_area.sort_by(|a, b| area(a).total_cmp(&area(b)));
        for (x, ox) in outers.iter().enumerate() {
            let Some(ox) = ox else { continue };
            let mut holders = by_area.iter().filter_map(|&p| Some((p, outers.get(p)?.as_ref()?))).filter(|&(p, op)| {
                BOX_TESTS.with(|n| n.set(n.get() + 1));
                let (o, i) = (op.bbox, ox.bbox);
                o.x0 - NEAR <= i.x0
                    && o.y0 - NEAR <= i.y0
                    && o.x1 + NEAR >= i.x1
                    && o.y1 + NEAR >= i.y1
                    && o.area() > i.area()
                    && !(stacked && p > x)
                    && !(o.x0 <= paper.x0 && o.y0 <= paper.y0 && o.x1 >= paper.x1 && o.y1 >= paper.y1)
            });
            let under = std::cell::OnceCell::new();
            let under = || {
                under.get_or_init(|| {
                    (0..x)
                        .filter(|&y| {
                            BOX_TESTS.with(|n| n.set(n.get() + 1));
                            outers.get(y).and_then(Option::as_ref).is_some_and(|oy| oy.bbox.intersect(ox.bbox).area() > 0.0)
                        })
                        .collect::<Vec<_>>()
                })
            };
            let found = holders.find(|&(p, op)| {
                ox.pts.first().is_some_and(|pt| inside_or_near(*pt, op))
                    && (!stacked || under().iter().all(|&y| !comes_after(&parent, &depth, y, p)))
                    && ox.pts.iter().all(|pt| inside_or_near(*pt, op))
            });
            if let Some((p, _)) = found {
                parent[x] = Some(p);
                depth[x] = depth[p] + 1;
            }
        }
        parent
    }

    #[test]
    fn the_grid_finds_the_parents_that_comparing_every_pair_finds() {
        let mut grouped = 0;
        for img in [rings(), faces(), noise(50, 50)] {
            for method in [crate::Method::Abutting, crate::Method::Overlapping] {
                let paths = traced(&img, method);
                // The image's own size, and sizes that leave paths outside it.
                for (w, h) in [(img.width as usize, img.height as usize), (40, 90), (0, 0)] {
                    for stacked in [false, true] {
                        let want = parents_by_pairs(&paths, w, h, stacked);
                        assert_eq!(parents(&paths, w, h, stacked), want, "{method:?} {w} x {h}, stacked {stacked}");
                        grouped += want.iter().filter(|p| p.is_some()).count();
                    }
                }
            }
        }
        assert!(grouped > 1000, "{grouped}");
    }

    #[test]
    fn a_dense_trace_is_grouped_with_a_few_box_tests_per_path() {
        let img = noise(150, 150);
        for (method, stacked) in [(crate::Method::Abutting, false), (crate::Method::Overlapping, true)] {
            let paths = traced(&img, method);
            let n = paths.len();
            let counted = |by_pairs: bool| {
                BOX_TESTS.with(|n| n.set(0));
                let found = if by_pairs { parents_by_pairs(&paths, 150, 150, stacked) } else { parents(&paths, 150, 150, stacked) };
                (found, BOX_TESTS.with(std::cell::Cell::get))
            };
            let (found, tests) = counted(false);
            let (want, pairs) = counted(true);
            assert!(n > 500, "{method:?}: {n} paths");
            assert_eq!(found, want, "{method:?}");
            assert!(tests <= 64 * n, "{method:?}: {tests} box tests for {n} paths");
            assert!(pairs > 64 * n, "{method:?}: {pairs} box tests by pairs for {n} paths");
        }
    }

    #[test]
    fn nested_paths_take_no_more_box_tests_than_comparing_every_pair() {
        // 300 discs on one center, each a little smaller than the one below it: every box
        // overlaps every other and reaches most of the grid.
        let paths: Vec<TracedPath> = (0..300).map(|i| disc((100.0, 100.0), 95.0 - f64::from(i) * 0.3, 0)).collect();
        for stacked in [false, true] {
            BOX_TESTS.with(|n| n.set(0));
            let found = parents(&paths, 200, 200, stacked);
            let tests = BOX_TESTS.with(std::cell::Cell::get);
            BOX_TESTS.with(|n| n.set(0));
            let want = parents_by_pairs(&paths, 200, 200, stacked);
            let pairs = BOX_TESTS.with(std::cell::Cell::get);
            assert_eq!(found, want, "stacked {stacked}");
            assert!(want.iter().skip(1).all(Option::is_some), "stacked {stacked}: {want:?}");
            assert!(tests <= pairs, "stacked {stacked}: {tests} box tests, {pairs} by pairs");
        }
    }

    #[test]
    fn boxes_that_reach_everywhere_make_the_cells_larger() {
        let boxes: Vec<(usize, Rect)> = (0..1000).map(|i| (i, Rect::new(0.0, 0.0, 1000.0, 1000.0))).collect();
        let grid = Grid::new(1000, 1000, boxes.iter().map(|&(_, r)| r));
        let listed: usize = grid.lists(&boxes).iter().map(Vec::len).sum();
        assert!(listed <= Grid::BUDGET * boxes.len(), "{listed}");
        // Small boxes keep about one cell each.
        let boxes: Vec<(usize, Rect)> =
            (0..1000).map(|i| (i, Rect::new(0.0, 0.0, 2.0, 2.0) + kurbo::Vec2::new((i % 40) as f64 * 25.0, (i / 40) as f64 * 40.0))).collect();
        let grid = Grid::new(1000, 1000, boxes.iter().map(|&(_, r)| r));
        assert!(grid.cols * grid.rows >= 900, "{} x {}", grid.cols, grid.rows);
    }

    /// Past the shared row budget, late contours keep grouping through the every-edge fallback:
    /// the parents are the ones an unlimited budget finds.
    #[test]
    fn grouping_survives_the_row_budget() {
        let mut paths = vec![];
        for i in 0..80 {
            let (cx, cy) = (40.0 + 80.0 * f64::from(i % 10), 40.0 + 80.0 * f64::from(i / 10));
            paths.push(disc((cx, cy), 30.0, 0));
        }
        // 80 discs about 62 rows each want about 4,960 row vectors, past the test cap of 4,096:
        // the last discs get no row index. A dot in the last disc still finds its parent.
        const { assert!(80 * 62 > MAX_ROWS) }; // the test shapes must cross the test cap
        paths.push(disc((760.0, 600.0), 3.0, 100));
        let found = parents(&paths, 810, 650, false);
        assert_eq!(found.last().copied().flatten(), Some(79));
        assert_eq!(found, parents_by_pairs(&paths, 810, 650, false));
    }
}
