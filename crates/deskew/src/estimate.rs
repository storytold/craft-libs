//! The two estimators: ink projection, and the edge vote for pages it cannot judge.

use crate::grid::{Grid, INK};
use crate::search::{MAX_POINTS, Point, search};

/// The fewest ink points a projection needs. Below it the page is blank or nearly so.
const MIN_INK_POINTS: usize = 64;

/// A Sobel response (|gx| + |gy| on 0–255 levels) below this is noise, not an edge: a quarter
/// of the response to a sharp step from paper to full ink, 4 × 255.
const MIN_EDGE: f64 = 255.0;

/// Project the grid's ink pixels. `None` when the ink is too little or too much to be lines:
/// less than two lines' worth along the grid's longer side is a lone rule or stray marks, whose
/// edges vote more reliably; more ink than paper means the ink is not drawn on the paper but
/// is the page, as in a photograph or a dark page.
pub(crate) fn projection(grid: &Grid) -> Option<(f64, f64)> {
    let ink = grid.levels.iter().filter(|&&l| l >= INK).count();
    let fewest = MIN_INK_POINTS.max(grid.width.max(grid.height).saturating_mul(2));
    if ink < fewest || ink.saturating_mul(2) >= grid.levels.len() {
        return None;
    }
    let stride = ink.div_ceil(MAX_POINTS).max(1);
    let width = grid.width.max(1);
    let points: Vec<Point> = grid
        .levels
        .iter()
        .enumerate()
        .filter(|&(_, &l)| l >= INK)
        .step_by(stride)
        .map(|(i, _)| Point { x: (i % width) as f64, y: (i / width) as f64, weight: 1.0 })
        .collect();
    search(&points)
}

/// Vote with the grid's edges, each weighted by its strength. Rules, frames and the tops and
/// bottoms of a few text lines line up even where there is too little ink to project, and a
/// photograph's horizon or a table border still votes when the page is mostly ink.
pub(crate) fn edges(grid: &Grid) -> Option<(f64, f64)> {
    let (w, h) = (grid.width, grid.height);
    if w < 3 || h < 3 {
        return None;
    }
    let interior = || (1..h - 1).flat_map(move |y| (1..w - 1).map(move |x| (x, y)));
    let strong = |&(x, y): &(usize, usize)| sobel(grid, x, y) >= MIN_EDGE;
    let count = interior().filter(strong).count();
    // Half a line's worth of edge along the longer side.
    if count < MIN_INK_POINTS.max(w.max(h) / 2) {
        return None;
    }
    // Counted first so the kept points spread over the whole page, not its top.
    let stride = count.div_ceil(MAX_POINTS).max(1);
    let found: Vec<Point> =
        interior().filter(strong).step_by(stride).map(|(x, y)| Point { x: x as f64, y: y as f64, weight: sobel(grid, x, y) }).collect();
    search(&found)
}

fn sobel(grid: &Grid, x: usize, y: usize) -> f64 {
    let at = |dx: usize, dy: usize| f64::from(grid.level(x + dx - 1, y + dy - 1));
    let gx = (at(2, 0) + 2.0 * at(2, 1) + at(2, 2)) - (at(0, 0) + 2.0 * at(0, 1) + at(0, 2));
    let gy = (at(0, 2) + 2.0 * at(1, 2) + at(2, 2)) - (at(0, 0) + 2.0 * at(1, 0) + at(2, 0));
    gx.abs() + gy.abs()
}
