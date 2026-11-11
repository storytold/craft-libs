//! A mask's boundary traced to outline segments, without egui.
//!
//! The selection overlay (`craft_imaging::ui::selection::SelectionOverlay`) draws marching ants
//! along these segments. The trace runs at display resolution: [`outline`] samples one pixel per
//! `step` × `step` block, so a 36 MP selection at 12 % zoom costs a 0.5 MP scan rather than a read
//! of the whole mask, and [`quantised_region`] keeps small pans on the cached trace.
//! [`CoverageGrid`] and [`edges_of_grid`] serve a boolean grid the app already holds (Content-Aware
//! Fill's fill area).
//!
//! The library has no raster type: the app implements [`MaskSource`] over its own mask (a
//! selection, a layer mask, a channel), as it implements `craft_ui::canvas::RasterSource` over its
//! composite. Every function here is serial; an app that wants the trace on another thread runs
//! [`outline`] in its own job and hands the overlay the segments.

use std::hash::{Hash, Hasher};

#[cfg(test)]
mod tests;

/// One boundary segment in document pixels, from one end to the other. Every segment is
/// horizontal or vertical.
pub type Segment = ([i32; 2], [i32; 2]);

/// The most cells a trace grid may have: [`bounded_step`] doubles the step until the grid fits.
// pc: crates/ui-egui/src/canvas/outline.rs:22 (1_048_576).
pub const TRACE_BUDGET: u64 = 1_048_576;

/// The most unit edges [`edges_of_grid`] returns before giving up.
// pc: crates/ui-egui/src/dialogs/caf_ui.rs:60 `MAX_OUTLINE_EDGES` (200_000).
pub const GRID_EDGE_LIMIT: usize = 200_000;

/// The finest and the coarsest trace step ([`trace_step`]).
// pc: crates/ui-egui/src/canvas.rs:248 (`clamp(1.0, 64.0)`).
pub const STEP_RANGE: (u32, u32) = (1, 64);

/// A rectangle of document pixels, `x0..x1` by `y0..y1` (the ends excluded): the egui-free twin of
/// `craft_ui::canvas::PixelRect`, in signed pixels because a mask's content may lie off the canvas
/// (a floating piece, a transformed selection).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaskRect {
    /// Left edge, included.
    pub x0: i32,
    /// Top edge, included.
    pub y0: i32,
    /// Right edge, excluded.
    pub x1: i32,
    /// Bottom edge, excluded.
    pub y1: i32,
}

impl MaskRect {
    /// The pixels `x0..x1` by `y0..y1`.
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The whole canvas of `size` pixels, from the origin.
    pub fn of_size(size: [u32; 2]) -> Self {
        let side = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
        Self::new(0, 0, side(size[0]), side(size[1]))
    }

    /// No pixel inside.
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    /// The width in pixels, 0 when empty.
    pub fn width(&self) -> u32 {
        u32::try_from(i64::from(self.x1) - i64::from(self.x0)).unwrap_or(0)
    }

    /// The height in pixels, 0 when empty.
    pub fn height(&self) -> u32 {
        u32::try_from(i64::from(self.y1) - i64::from(self.y0)).unwrap_or(0)
    }

    /// Whether pixel `(x, y)` is inside.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// The part inside both; empty (the default) when they do not overlap.
    pub fn intersect(&self, other: &MaskRect) -> MaskRect {
        let r = MaskRect::new(self.x0.max(other.x0), self.y0.max(other.y0), self.x1.min(other.x1), self.y1.min(other.y1));
        if r.is_empty() { MaskRect::default() } else { r }
    }
}

/// A coverage mask the app owns: the selection, a layer mask, a channel, a proxy's refined
/// selection. Coverage is 0 (unselected) to 1 (selected); the trace's threshold is 0.5.
///
/// The app implements it over its own storage; PhotoCraft's is about 30 lines over its tiled
/// `Surface` (`sample_row_strided`, `sample_channel`).
pub trait MaskSource {
    /// The document's size in pixels: the canvas the mask belongs to.
    fn size(&self) -> [u32; 2];

    /// Changes whenever the coverage changes. The overlay retraces only when it, the step or the
    /// region changes, so an edit elsewhere in the document (a brush stroke on a layer) keeps the
    /// trace: key it on the mask, not on the document's revision (PhotoCraft fingerprints the
    /// mask's tiles).
    fn revision(&self) -> u64;

    /// The coverage everywhere outside [`content_bounds`](Self::content_bounds): 0 for a
    /// selection, 1 for an inverted one or a reveal-all layer mask.
    fn outside(&self) -> f32 {
        0.0
    }

    /// The bounds of the pixels whose coverage differs from [`outside`](Self::outside); empty for
    /// an empty mask. The trace reads nothing beyond it.
    fn content_bounds(&self) -> MaskRect;

    /// Writes the coverage of row `y` at `x0`, `x0 + step`, `x0 + 2 · step`, … into `out` (one
    /// value per element of `out`). Pixels outside the content read [`outside`](Self::outside).
    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]);

    /// The coverage of one pixel. Default: a one-element [`read_row`](Self::read_row).
    fn at(&self, x: i32, y: i32) -> f32 {
        let mut v = [0.0];
        self.read_row(y, x, 1, &mut v);
        v[0]
    }
}

/// The trace step for a view showing `device_zoom` device pixels per document pixel of a mask
/// reduced `reduction` times (Select and Mask's preview proxy): the largest power of two at or
/// below `1 / (device_zoom · reduction)`, within [`STEP_RANGE`]. Device pixels, not points, so a
/// one-pixel selection still traces at 100 % on a scaled display.
pub fn trace_step(device_zoom: f64, reduction: u32) -> u32 {
    let scale = (device_zoom * f64::from(reduction.max(1))).max(1e-3);
    let step = (1.0 / scale).log2().floor().exp2().clamp(f64::from(STEP_RANGE.0), f64::from(STEP_RANGE.1));
    step as u32
}

/// The region of a `reduction`-times reduced mask to trace for the document rectangle `visible`
/// (`[x0, y0, x1, y1]` in full-size document units, the view's visible bounds): divided by
/// `reduction`, rounded out to multiples of `q = 256 · step` and padded by one `q` before and two
/// after, so pans of less than `q` keep the same region and the cached trace. Coordinates are
/// clamped to ±2³⁰ first (a view centre may be any finite number), so the padding cannot overflow.
pub fn quantised_region(visible: [f64; 4], step: u32, reduction: u32) -> MaskRect {
    let k = f64::from(reduction.max(1));
    let q = 256 * i64::from(step.clamp(STEP_RANGE.0, STEP_RANGE.1));
    let px = |v: f64| v.clamp(-f64::from(1_u32 << 30), f64::from(1_u32 << 30)) as i64;
    let lo = |v: f64| px((v / k).floor()).div_euclid(q) * q - q;
    let hi = |v: f64| (px((v / k).ceil()).div_euclid(q) + 2) * q;
    let side = |v: i64| i32::try_from(v).unwrap_or(i32::MAX);
    MaskRect::new(side(lo(visible[0])), side(lo(visible[1])), side(hi(visible[2])), side(hi(visible[3])))
}

/// `step` doubled until the trace grid over `bounds` (rounded out to whole cells, plus the two
/// padding cells on each axis) has at most [`TRACE_BUDGET`] cells. A step below 1 counts as 1.
/// Used when the whole selection is traced (a Free Transform carries it, so parts outside the view
/// may come into it), never for an ordinary visible-region trace.
pub fn bounded_step(bounds: MaskRect, step: u32) -> u32 {
    let mut step = step.max(1);
    let cells = |step: u32| {
        let s = i64::from(step);
        let axis = |lo: i32, hi: i32| u64::try_from(((i64::from(hi) + s - 1).div_euclid(s) - i64::from(lo).div_euclid(s) + 2).max(0)).unwrap_or(0);
        axis(bounds.x0, bounds.x1).saturating_mul(axis(bounds.y0, bounds.y1))
    };
    while cells(step) > TRACE_BUDGET {
        step = step.saturating_mul(2);
    }
    step
}

/// The boundary of `mask` inside `bounds` (coverage above 0.5 is inside), sampling one pixel per
/// `step` × `step` block, as merged horizontal runs followed by merged vertical runs.
///
/// The grid is padded by one cell on every side, so edges on `bounds` are found. Each cell samples
/// its centre pixel; a row's sample `y` is clamped into `bounds`. A cell whose centre falls outside
/// `bounds` but which overlaps it samples the nearest pixel inside (one [`MaskSource::at`]), so a
/// one-pixel selection still shows at a coarse step; padding cells stay outside. Segment ends are
/// clamped to `bounds`, so a coarse cell never draws outside the selection. Empty `bounds` give no
/// segments.
pub fn outline<M: MaskSource + ?Sized>(mask: &M, bounds: MaskRect, step: u32) -> Vec<Segment> {
    if bounds.is_empty() {
        return Vec::new();
    }
    let step = i64::from(step.max(1));
    let half = step / 2;
    let (bx0, by0, bx1, by1) = (i64::from(bounds.x0), i64::from(bounds.y0), i64::from(bounds.x1), i64::from(bounds.y1));
    // The grid of block samples, padded by one cell on every side so edges on the bounds are found.
    let gx0 = bx0.div_euclid(step) - 1;
    let gy0 = by0.div_euclid(step) - 1;
    let gx1 = (bx1 + step - 1).div_euclid(step) + 1;
    let gy1 = (by1 + step - 1).div_euclid(step) + 1;
    let (w, h) = (usize::try_from(gx1 - gx0).unwrap_or(0), usize::try_from(gy1 - gy0).unwrap_or(0));
    let clamp_px = |v: i64, lo: i64, hi: i64| i32::try_from(v.clamp(lo, hi)).unwrap_or(0);
    let stride = u32::try_from(step).unwrap_or(u32::MAX);
    let mut grid = vec![false; w * h];
    let mut vals = vec![0.0_f32; w];
    for (gy, row) in grid.chunks_mut(w).enumerate() {
        let y0 = (gy0 + gy as i64) * step;
        if y0 >= by1 || y0 + step <= by0 {
            continue;
        }
        // A coarse cell can overlap a one-pixel selection although its centre misses it; the
        // sample row is clamped into the bounds. Padding cells stay outside.
        let y = (y0 + half).clamp(by0, by1 - 1);
        let first = gx0 * step + half;
        mask.read_row(clamp_px(y, by0, by1 - 1), clamp_px(first, i64::from(i32::MIN), i64::from(i32::MAX)), stride, &mut vals);
        for (gx, (cell, v)) in row.iter_mut().zip(&vals).enumerate() {
            let x = (gx0 + gx as i64) * step + half;
            *cell = if x >= bx0 && x < bx1 {
                *v > 0.5
            } else {
                x - half < bx1 && x - half + step > bx0 && mask.at(clamp_px(x, bx0, bx1 - 1), clamp_px(y, by0, by1 - 1)) > 0.5
            };
        }
    }
    let inside = |x: usize, y: usize| grid[y * w + x];
    // Grid to document coordinates, clamped so a coarse cell never draws outside the bounds.
    let cx = |gx: usize| clamp_px((gx0 + gx as i64) * step, bx0, bx1);
    let cy = |gy: usize| clamp_px((gy0 + gy as i64) * step, by0, by1);
    let mut segments = Vec::new();
    // Horizontal edges between rows y - 1 and y, merged along x.
    for y in 1..h {
        let mut run: Option<usize> = None;
        for x in 0..=w {
            let edge = x < w && inside(x, y) != inside(x, y - 1);
            match (edge, run) {
                (true, None) => run = Some(x),
                (false, Some(start)) => {
                    let yy = cy(y);
                    segments.push(([cx(start), yy], [cx(x), yy]));
                    run = None;
                }
                _ => {}
            }
        }
    }
    // Vertical edges between columns x - 1 and x, merged along y.
    for x in 1..w {
        let mut run: Option<usize> = None;
        for y in 0..=h {
            let edge = y < h && inside(x, y) != inside(x - 1, y);
            match (edge, run) {
                (true, None) => run = Some(y),
                (false, Some(start)) => {
                    let xx = cx(x);
                    segments.push(([xx, cy(start)], [xx, cy(y)]));
                    run = None;
                }
                _ => {}
            }
        }
    }
    segments
}

/// A boolean grid of document pixels the app holds already: a cell is inside or not. Content-Aware
/// Fill's fill area and sampling area are such grids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageGrid {
    origin: [i32; 2],
    size: [u32; 2],
    cells: Vec<bool>,
    /// Hash of the origin, size and cells, computed once in `new`.
    hash: u64,
    /// The bounds of the inside cells; empty when none is inside.
    bounds: MaskRect,
}

impl CoverageGrid {
    /// A grid of `size` cells whose top-left cell is document pixel `origin`; `cells` row by row,
    /// `size[0] · size[1]` of them. Missing cells are outside, extra ones are ignored.
    pub fn new(origin: [i32; 2], size: [u32; 2], cells: Vec<bool>) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        origin.hash(&mut hasher);
        size.hash(&mut hasher);
        cells.hash(&mut hasher);
        let hash = hasher.finish();
        let width = size[0] as usize;
        let mut found: Option<[usize; 4]> = None;
        if width > 0 {
            let rows = cells.chunks(width).take(size[1] as usize);
            for (gy, row) in rows.enumerate() {
                let mut inside = row.iter().enumerate().filter(|(_, c)| **c).map(|(gx, _)| gx);
                let (Some(first), Some(last)) = (inside.clone().next(), inside.next_back()) else { continue };
                found = Some(match found {
                    None => [first, gy, last, gy],
                    Some([x0, y0, x1, _]) => [x0.min(first), y0, x1.max(last), gy],
                });
            }
        }
        let at = |o: i32, n: usize| i32::try_from((i64::from(o) + n as i64).clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(i32::MAX);
        let bounds = found.map_or_else(MaskRect::default, |[x0, y0, x1, y1]| {
            MaskRect::new(at(origin[0], x0), at(origin[1], y0), at(origin[0], x1 + 1), at(origin[1], y1 + 1))
        });
        Self { origin, size, cells, hash, bounds }
    }

    /// The document pixel of the top-left cell.
    pub fn origin(&self) -> [i32; 2] {
        self.origin
    }

    /// The grid's width and height in cells.
    pub fn grid_size(&self) -> [u32; 2] {
        self.size
    }

    /// Whether document pixel `(x, y)` is inside; `false` off the grid.
    pub fn inside(&self, x: i32, y: i32) -> bool {
        let (dx, dy) = (i64::from(x) - i64::from(self.origin[0]), i64::from(y) - i64::from(self.origin[1]));
        let (Ok(dx), Ok(dy)) = (u32::try_from(dx), u32::try_from(dy)) else { return false };
        if dx >= self.size[0] || dy >= self.size[1] {
            return false;
        }
        let index = u64::from(dy) * u64::from(self.size[0]) + u64::from(dx);
        usize::try_from(index).ok().and_then(|i| self.cells.get(i)).copied().unwrap_or(false)
    }
}

/// A grid as a mask: inside cells read 1, the rest 0. Its revision is a hash of the cells, taken
/// once when the grid is built, so an overlay retraces only when they change. Its content bounds
/// are those of the inside cells, empty when none is inside.
impl MaskSource for CoverageGrid {
    fn size(&self) -> [u32; 2] {
        let far = |o: i32, n: u32| u32::try_from((i64::from(o) + i64::from(n)).max(0)).unwrap_or(u32::MAX);
        [far(self.origin[0], self.size[0]), far(self.origin[1], self.size[1])]
    }

    fn revision(&self) -> u64 {
        self.hash
    }

    fn content_bounds(&self) -> MaskRect {
        self.bounds
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        for (i, v) in out.iter_mut().enumerate() {
            let x = i64::from(x0) + i as i64 * i64::from(step);
            *v = match i32::try_from(x) {
                Ok(x) if self.inside(x, y) => 1.0,
                _ => 0.0,
            };
        }
    }
}

/// Every unit edge between an inside cell of `grid` and an outside one (or the grid's border), in
/// document pixels, without merging into runs: per inside cell in row order, its left, right, top
/// and bottom edge where the neighbour is outside. `None` when there are more than `max_edges`
/// (callers pass [`GRID_EDGE_LIMIT`]); the caller then draws no outline.
pub fn edges_of_grid(grid: &CoverageGrid, max_edges: usize) -> Option<Vec<Segment>> {
    let [w, h] = grid.grid_size();
    let [ox, oy] = grid.origin();
    let mut out = Vec::new();
    for gy in 0..h {
        for gx in 0..w {
            let (x, y) = (ox.saturating_add_unsigned(gx), oy.saturating_add_unsigned(gy));
            if !grid.inside(x, y) {
                continue;
            }
            if !grid.inside(x - 1, y) {
                out.push(([x, y], [x, y + 1]));
            }
            if !grid.inside(x + 1, y) {
                out.push(([x + 1, y], [x + 1, y + 1]));
            }
            if !grid.inside(x, y - 1) {
                out.push(([x, y], [x + 1, y]));
            }
            if !grid.inside(x, y + 1) {
                out.push(([x, y + 1], [x + 1, y + 1]));
            }
            if out.len() > max_edges {
                return None;
            }
        }
    }
    Some(out)
}
