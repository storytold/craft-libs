use super::*;

/// The sweep's frame: y + x / 32. Only edges sloping -1/32 in the document are horizontal in it.
pub(super) const SHEAR: Affine = Affine::new([1.0, 1.0 / 32.0, 0.0, 1.0, 0.0, 0.0]);

pub(super) const UNSHEAR: Affine = Affine::new([1.0, -1.0 / 32.0, 0.0, 1.0, 0.0, 0.0]);

/// Boundary crossings per input, counted from the far west: slot `2i` counts input `i`'s
/// segments running one way, slot `2i + 1` the other. An open path traced there and back adds one
/// to each, so its edges never cancel out of the map. Only the slots that aren't 0 are kept, in
/// order: the sweep keeps one of these for every edge, and a vector as long as the input's number
/// (as this was) made disjoint shapes cost the square of their count in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Crossings(pub(super) Vec<(usize, u32)>);

impl Crossings {
    pub(super) fn get(&self, slot: usize) -> u32 {
        self.0.binary_search_by_key(&slot, |(s, _)| *s).map_or(0, |i| self.0.get(i).map_or(0, |(_, c)| *c))
    }
}

impl std::ops::Add for Crossings {
    type Output = Crossings;
    fn add(mut self, rhs: Self) -> Crossings {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Crossings {
    fn add_assign(&mut self, rhs: Self) {
        if rhs.0.is_empty() {
            return;
        }
        let mut out = Vec::with_capacity(self.0.len() + rhs.0.len());
        let (mut a, mut b) = (std::mem::take(&mut self.0).into_iter().peekable(), rhs.0.into_iter().peekable());
        loop {
            let next = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) if x.0 == y.0 => {
                    let (x, y) = (a.next(), b.next());
                    x.zip(y).map(|(x, y)| (x.0, x.1.saturating_add(y.1)))
                }
                (Some(x), Some(y)) if x.0 < y.0 => a.next(),
                (Some(_), Some(_)) | (None, Some(_)) => b.next(),
                (Some(_), None) => a.next(),
                (None, None) => break,
            };
            out.extend(next.filter(|(_, c)| *c != 0));
        }
        self.0 = out;
    }
}

impl WindingNumber for Crossings {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        Crossings(vec![(2 * tag + usize::from(!positive), 1)])
    }
    fn of_tag(&self, tag: usize) -> Self {
        Crossings([2 * tag, 2 * tag + 1].into_iter().map(|slot| (slot, self.get(slot))).filter(|(_, c)| *c != 0).collect())
    }
}

#[cfg(test)]
thread_local! {
    /// [`work_done`] of this thread.
    static WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many items [`RectIndex::at`] has read and segments [`run_around`](super::run_around) has
/// searched for the nearest point on a whole piece, on this thread.
#[cfg(test)]
pub(super) fn work_done() -> u64 {
    WORK.get()
}

/// Adds `n` to [`work_done`].
#[cfg(test)]
pub(super) fn note_work(n: usize) {
    WORK.set(WORK.get().saturating_add(n as u64));
}

/// A point in the middle of a run of segments.
pub(super) fn mid(segs: &[Seg]) -> Option<Point> {
    segs.get(segs.len() / 2).map(|s| s.c.eval(0.5))
}

/// Rectangles looked up by a point inside them, when there are too many to test one by one for
/// every point (the map asks once per edge and once per hole, so a scan of them all made it
/// quadratic in the input, #937). A grid of cells over their bounds, each cell listing the
/// rectangles touching it; a lookup reads the cells around the point. The grid has about one cell
/// per rectangle, so a lookup reads about the rectangles of the point's neighbourhood. A rectangle
/// covering too many cells (a long diagonal's bounds take in most of the art, so an index of the
/// bounds alone scanned a diagonal hatch whole for every point) is filed under the cells of a
/// smaller footprint instead ([`Self::with_footprint`]), or, failing that, scanned for every point.
pub(super) struct RectIndex<T> {
    items: Vec<(kurbo::Rect, T)>,
    /// The grid's top-left corner, cell size and column count; `cells` lists the items touching
    /// each cell, row by row.
    origin: Point,
    cell: (f64, f64),
    cols: usize,
    pub(super) cells: Vec<Vec<u32>>,
    /// Items the grid doesn't hold (bounds covering most of it, or not finite), scanned for
    /// every point.
    pub(super) rest: Vec<usize>,
}

/// The fewest cells one footprint rectangle may always be filed under. The cap is this or two
/// rows or columns of the grid, whichever is more: a thin stripe across the art (one of the faces
/// a hatch cuts a shape into) is filed under the cells along it, while a rectangle covering most
/// of the grid costs more to file and to read than to scan for every point.
const CELLS_PER_FOOTPRINT: usize = 64;

/// The cell references a grid of `cells` cells holds for `n` items before the remaining items go
/// to the scanned list: a few per item, plus a few rows or columns' worth (a long segment is filed
/// under a cell of each row or column it crosses, about `sqrt(cells)` of them in a square grid).
/// Coincident segments make a grid of one column and a row per segment, where each would be filed
/// under every row: a quadratic number of references, which this keeps to `n·sqrt(n)`.
pub(super) fn reference_budget(n: usize, cells: usize) -> usize {
    let across = (cells as f64).sqrt().ceil().min(1e9) as usize;
    n.saturating_mul(64 + 4 * across).saturating_add(1024)
}

/// The cells (columns or rows) an interval `lo..=hi` touches on one axis of a grid: clamped to the
/// grid, so an interval past its edge gets the nearest cell (whose items `contains` then rejects),
/// and `None` for an interval that isn't a number.
fn cell_span(lo: f64, hi: f64, origin: f64, size: f64, count: usize) -> Option<std::ops::RangeInclusive<usize>> {
    let last = count.saturating_sub(1) as f64;
    let a = ((lo - origin) / size).floor().clamp(0.0, last);
    let b = ((hi - origin) / size).floor().clamp(0.0, last);
    (a.is_finite() && b.is_finite()).then(|| (a as usize)..=(b.max(a) as usize))
}

impl<T> RectIndex<T> {
    pub(super) fn new(items: Vec<(kurbo::Rect, T)>) -> Self {
        Self::with_footprint(items, |r, _, _| vec![*r])
    }

    /// `footprint(bounds, item, cell side)` gives the rectangles an item is filed under: a lookup
    /// finds the item when its bounds hold the point and one footprint rectangle is within the
    /// lookup's tolerance of it. The bounds themselves (`new`) suit items as compact as their
    /// bounds; a long diagonal segment is better filed under the bounds of its pieces, about a
    /// cell long each, which take in the cells along it rather than every cell its bounds cover.
    pub(super) fn with_footprint(items: Vec<(kurbo::Rect, T)>, footprint: impl Fn(&kurbo::Rect, &T, f64) -> Vec<kurbo::Rect>) -> Self {
        let n = items.len().max(1);
        let all = items.iter().map(|(r, _)| *r).filter(kurbo::Rect::is_finite).reduce(|a, b| a.union(b)).unwrap_or(kurbo::Rect::ZERO);
        // A cell's side holds one rectangle on average; a grid of collinear rectangles (zero area)
        // is divided along its length.
        let side = (all.area() / n as f64).sqrt();
        let side = if side > 0.0 { side } else { all.width().max(all.height()) / n as f64 }.max(1e-9);
        // Saturating casts: an extent of a billion cells is clamped to the item count, then the
        // grid is kept to a few cells per item however lopsided its bounds.
        let mut cols = ((all.width() / side).ceil() as usize).clamp(1, 2 * n);
        let mut rows = ((all.height() / side).ceil() as usize).clamp(1, 2 * n);
        while cols * rows > 4 * n + 4 {
            if cols > rows { cols = cols.div_ceil(2) } else { rows = rows.div_ceil(2) }
        }
        let cell = ((all.width() / cols as f64).max(1e-9), (all.height() / rows as f64).max(1e-9));
        let cap = CELLS_PER_FOOTPRINT.max(2 * cols.max(rows));
        let mut budget = reference_budget(n, cols * rows);
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); cols * rows];
        let mut rest = Vec::new();
        for (i, (r, t)) in items.iter().enumerate() {
            // Cells hold `u32` indices to halve the memory of a long stripe filed under a row of them.
            let Ok(index) = u32::try_from(i) else {
                rest.push(i);
                continue;
            };
            let mut scan = !r.is_finite();
            let mut spans = Vec::new();
            for f in if scan { Vec::new() } else { footprint(r, t, cell.0.min(cell.1)) } {
                match (cell_span(f.x0, f.x1, all.x0, cell.0, cols), cell_span(f.y0, f.y1, all.y0, cell.1, rows)) {
                    (Some(xs), Some(ys)) if (xs.end() - xs.start() + 1) * (ys.end() - ys.start() + 1) <= cap => spans.push((xs, ys)),
                    _ => scan = true,
                }
            }
            // The item's cells, counted before they are filed: past the grid's budget the item,
            // and the ones after it, are scanned instead (the index stays bounded in memory).
            // Pieces of one segment that fall in the same cell (a degenerate grid's tiny cells)
            // count and file it once: the pieces come in order, so the repeats are consecutive.
            let mut mine: Vec<usize> = if scan {
                Vec::new()
            } else {
                spans.iter().flat_map(|(xs, ys)| ys.clone().flat_map(|y| xs.clone().map(move |x| y * cols + x))).collect()
            };
            mine.dedup();
            if !scan && mine.len() > budget {
                scan = true;
            }
            if scan {
                rest.push(i);
                continue;
            }
            budget -= mine.len();
            for c in mine {
                if let Some(c) = cells.get_mut(c)
                    && c.last() != Some(&index)
                {
                    c.push(index);
                }
            }
        }
        Self { items, origin: all.origin(), cell, cols, cells, rest }
    }

    /// The items whose rectangle, grown by `tol`, holds `p` (and, with a footprint, one of whose
    /// footprint rectangles is within `tol` of it).
    pub(super) fn at(&self, p: Point, tol: f64) -> impl Iterator<Item = &T> {
        let mut found = self.rest.clone();
        let rows = self.cells.len() / self.cols.max(1);
        if let (Some(xs), Some(ys)) = (
            cell_span(p.x - tol, p.x + tol, self.origin.x, self.cell.0, self.cols),
            cell_span(p.y - tol, p.y + tol, self.origin.y, self.cell.1, rows),
        ) {
            for y in ys {
                for x in xs.clone() {
                    found.extend(self.cells.get(y * self.cols + x).into_iter().flatten().map(|&i| i as usize));
                }
            }
        }
        #[cfg(test)]
        note_work(found.len());
        found.sort_unstable();
        found.dedup();
        found.into_iter().filter_map(move |i| self.items.get(i)).filter(move |(r, _)| r.inflate(tol, tol).contains(p)).map(|(_, t)| t)
    }
}

/// A segment's footprint in a [`RectIndex`]: the bounds of its pieces about a cell long, so a long
/// diagonal is filed under the cells along it, not under every cell its bounds cover.
pub(super) fn segment_footprint(bounds: &kurbo::Rect, seg: &PathSeg, cell: f64) -> Vec<kurbo::Rect> {
    let pieces = (bounds.width().max(bounds.height()) / cell).ceil();
    // A count past the cap gives longer pieces, each filed under more cells.
    if pieces.is_nan() || pieces <= 1.0 {
        return vec![*bounds];
    }
    let pieces = pieces.min(4096.0) as usize;
    (0..pieces).map(|k| seg.subsegment(k as f64 / pieces as f64..(k + 1) as f64 / pieces as f64).bounding_box()).collect()
}
