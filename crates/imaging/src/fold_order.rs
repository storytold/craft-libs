//! The cell order of a warp: one order for the preview and the render, without egui or kurbo.

/// The order the cells of a warp mesh row `row` are drawn in (`row` vertices a row; `pos` where the warp put each
/// vertex and `rest` where it would be without it, as `[x, y]` pairs).
///
/// Where a warp folds the picture over itself, the part turned over lies on top, like a folded-back flap: a corner or
/// edge dragged across the picture, and also the edge row that folds back when the middle is pulled out past it, which
/// leaves the pulled part under that row. Within each, the cells moved least come first, so the part dragged furthest
/// is on top. A vertex with no finite movement (sent to infinity, as near a Distort's horizon) goes under, not on top.
/// Each cell is named by its top-left vertex. Mismatched lengths or a `row` below 2 give no cells.
pub fn cell_order(pos: &[[f64; 2]], rest: &[[f64; 2]], row: usize) -> Vec<usize> {
    if row < 2 || pos.len() != rest.len() {
        return Vec::new();
    }
    let rows = pos.len() / row;
    let moved = |k: usize| match (pos.get(k), rest.get(k)) {
        (Some(p), Some(r)) => Some((p[0] - r[0]).hypot(p[1] - r[1])).filter(|m| m.is_finite()).unwrap_or(f64::NEG_INFINITY),
        _ => f64::NEG_INFINITY,
    };
    // Twice the signed area of a cell's quad: its sign is the cell's orientation (NaN when a corner isn't finite,
    // which counts as not turned over).
    let area = |pts: &[[f64; 2]], a: usize| {
        let q = [a, a + 1, a + row + 1, a + row].map(|k| pts.get(k).copied().unwrap_or([f64::NAN; 2]));
        (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum::<f64>()
    };
    let mut cells: Vec<(bool, f64, usize)> = Vec::with_capacity(rows.saturating_sub(1) * (row - 1));
    for j in 0..rows.saturating_sub(1) {
        for i in 0..row - 1 {
            let a = j * row + i;
            let turned = area(pos, a) * area(rest, a) < 0.0;
            cells.push((turned, moved(a) + moved(a + 1) + moved(a + row) + moved(a + row + 1), a));
        }
    }
    let by = |x: &(bool, f64, usize), y: &(bool, f64, usize)| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1));
    // A stable sort, so equal cells keep row order. On this thread alone: a render asks for the order from a pool
    // thread that other tiles wait on, and a sort shared with the pool could hand that thread one of those tiles,
    // which would wait for the order for ever.
    cells.sort_by(by);
    cells.into_iter().map(|(_, _, a)| a).collect()
}
