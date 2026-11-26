//! Slice rectangles and the rules that size them: [`fixed_ratio_pixels`] (a Fixed Ratio slice
//! from a drag) and [`resized_rect`] (a handle drag). No egui. Normal and Fixed Size slices use the
//! marquee's rules (`craft_behavior::geometry::{Constraint::corners, marquee_pixels, fixed_size_pixels}`).
//!
//! Slices are whole document pixels: every rectangle these return has integer edges.

use kurbo::{Point, Rect, Vec2};

/// The handles of a rectangle in hit order, corners first, then edge middles: each is the
/// direction from the centre per axis (−1 the low edge, 0 the middle, 1 the high edge).
/// PhotoCraft's slice handles and the artboard handles use this order.
pub const HANDLES: [[i8; 2]; 8] = [[-1, -1], [1, -1], [-1, 1], [1, 1], [0, -1], [0, 1], [-1, 0], [1, 0]];

/// The Fixed Ratio slice drawn from `start` to `end` with the ratio `(w, h)`: the longer side of
/// the ratio follows the drag to within a pixel and the shorter side takes the ratio from it,
/// rounded to whole pixels (at least 1). A drag shorter than half a pixel along the longer side is
/// a click: a slice of no size. The slice sits on the pixel the press is in: drawn right or down
/// it starts at that pixel's top-left edge, drawn left or up at its far edge, so it holds that
/// pixel either way.
///
/// `Constraint::Ratio` is not used: rounding each edge of a fractional drag apart turns 2 : 1
/// into 5 × 3 (PhotoCraft test `fixed_ratio_slices_keep_their_ratio_from_a_fractional_drag`).
pub fn fixed_ratio_pixels(start: Point, end: Point, ratio: (f64, f64)) -> Rect {
    // pc: crates/ui-egui/src/tools/slice_ui.rs:113-148 `new_rect` ("fixedRatio": `k = |end.x -
    // start.x| / w` after `marquee_end`, `follow(long, short)` with `l = (k * long).round()` and
    // `(l * short / long).round().max(1.0)`; the corner on the press's pixel at :143-147).
    let (w, h) = ratio;
    if w <= 0.0 || h <= 0.0 {
        return pixels(start, end);
    }
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let k = (dx.abs() / w).max(dy.abs() / h);
    // A drag shorter than half a pixel along the longer side is a click: no size.
    let follow = |long: f64, short: f64| {
        let l = (k * long).round();
        [l, if l > 0.0 { (l * short / long).round().max(1.0) } else { 0.0 }]
    };
    let size = if w >= h {
        follow(w, h)
    } else {
        let [l, s] = follow(h, w);
        [s, l]
    };
    let sign = |d: f64| if d < 0.0 { -1.0 } else { 1.0 };
    let dir = [sign(dx), sign(dy)];
    let s = [start.x, start.y];
    let corner = [0, 1].map(|i| if dir[i] < 0.0 { s[i].ceil() } else { s[i].floor() });
    pixels(Point::new(corner[0], corner[1]), Point::new(corner[0] + dir[0] * size[0], corner[1] + dir[1] * size[1]))
}

/// `rect` with the edges `handle` grabs (one of [`HANDLES`]) moved by `by`, rounded to whole
/// pixels; an edge dragged past the opposite one turns the rectangle the right way round, which
/// stays at least 1 × 1.
pub fn resized_rect(rect: Rect, handle: [i8; 2], by: Vec2) -> Rect {
    // pc: crates/ui-egui/src/tools/slice_ui.rs:78-93 `resized` (`x0 + dx` for handle −1, `x1 + dx`
    // for 1; `(x0.min(x1), x0.max(x1).max(x0.min(x1) + 1))` per axis).
    let (dx, dy) = (by.x.round(), by.y.round());
    let (mut x0, mut y0, mut x1, mut y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    match handle[0] {
        -1 => x0 += dx,
        1 => x1 += dx,
        _ => {}
    }
    match handle[1] {
        -1 => y0 += dy,
        1 => y1 += dy,
        _ => {}
    }
    let (x0, x1) = (x0.min(x1), x0.max(x1).max(x0.min(x1) + 1.0));
    let (y0, y1) = (y0.min(y1), y0.max(y1).max(y0.min(y1) + 1.0));
    Rect::new(x0, y0, x1, y1)
}

/// The handle of `rect` within `reach` document units of `point`, in [`HANDLES`] order (corners
/// win over edge middles), or `None`.
pub fn handle_at(rect: Rect, point: Point, reach: f64) -> Option<[i8; 2]> {
    // pc: crates/ui-egui/src/tools/slice_ui.rs:58-74 `handle_at` (a square reach per axis around each
    // handle; corners first).
    let (x0, y0, x1, y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    HANDLES.into_iter().find(|h| {
        let x = match h[0] {
            -1 => x0,
            1 => x1,
            _ => (x0 + x1) / 2.0,
        };
        let y = match h[1] {
            -1 => y0,
            1 => y1,
            _ => (y0 + y1) / 2.0,
        };
        (point.x - x).abs() <= reach && (point.y - y).abs() <= reach
    })
}

/// The whole pixels a drag between two corners covers, edges snapped outwards (the marquee's
/// rule; craft-ui's `marquee_pixels` is not reachable without the `ui` feature).
fn pixels(a: Point, b: Point) -> Rect {
    Rect::new(a.x.min(b.x).floor(), a.y.min(b.y).floor(), a.x.max(b.x).ceil(), a.y.max(b.y).ceil())
}
