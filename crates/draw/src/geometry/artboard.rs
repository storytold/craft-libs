//! Artboard rectangles and the rules that size them: [`drawn_rect`], [`moved_rect`],
//! [`resized_rect`] and [`resized_keep_ratio`]. No egui.
//!
//! Artboards are whole document pixels: every rectangle these return has integer edges and is at
//! least 1 × 1. A handle is one of [`HANDLES`](super::slice::HANDLES) (−1, 0 or 1 per axis).

use kurbo::{Point, Rect, Vec2};

/// The board a drag from `a` to `b` draws: both corners rounded to whole pixels, ordered.
pub fn drawn_rect(a: Point, b: Point) -> Rect {
    // pc: crates/ui-egui/src/tools/artboard_tool.rs:113-115 `drawn`.
    Rect::new(a.x.min(b.x).round(), a.y.min(b.y).round(), a.x.max(b.x).round(), a.y.max(b.y).round())
}

/// `rect` moved by `by` rounded to whole pixels (a move keeps the size).
pub fn moved_rect(rect: Rect, by: Vec2) -> Rect {
    // pc: crates/ui-egui/src/tools/artboard_tool.rs:118-120 `offset` (each axis rounded), :158-161.
    rect + Vec2::new(by.x.round(), by.y.round())
}

/// `rect` with handle `handle` dragged by `by` (rounded to whole pixels). An edge dragged past the
/// opposite one flips the board, which never gets narrower than one pixel. `about_centre` (⌥)
/// moves the opposite edge by the same amount the other way, so the centre stays.
pub fn resized_rect(rect: Rect, handle: [i8; 2], by: Vec2, about_centre: bool) -> Rect {
    // pc: crates/ui-egui/src/tools/artboard_tool.rs:125-139 `resized` (per axis: −1 moves the low
    // edge, 1 the high one; `(a, b.max(a + 1))`).
    // new: `about_centre` (⌥, Photoshop's artboard resize; PhotoCraft has none).
    let (dx, dy) = (by.x.round(), by.y.round());
    let mut e = [rect.x0, rect.y0, rect.x1, rect.y1];
    for (h, lo, hi, dv) in [(handle[0], 0, 2, dx), (handle[1], 1, 3, dy)] {
        match h {
            -1 => {
                e[lo] += dv;
                if about_centre {
                    e[hi] -= dv;
                }
            }
            1 => {
                e[hi] += dv;
                if about_centre {
                    e[lo] -= dv;
                }
            }
            _ => {}
        }
        let (a, b) = (e[lo].min(e[hi]), e[lo].max(e[hi]));
        (e[lo], e[hi]) = (a, b.max(a + 1.0));
    }
    Rect::new(e[0], e[1], e[2], e[3])
}

/// `rect` with corner `handle` dragged by `by`, keeping its proportions (⇧ on a corner): the
/// opposite corner stays (the centre with `about_centre`), and the board scales by the larger of
/// the two axes' factors, each side rounded to whole pixels and at least 1. An edge handle (a 0 in
/// `handle`) resizes as [`resized_rect`] does.
pub fn resized_keep_ratio(rect: Rect, handle: [i8; 2], by: Vec2, about_centre: bool) -> Rect {
    // pc: crates/ui-egui/src/tools/artboard_tool.rs:141-153 `resized_keep` (`k = (dw.abs() /
    // w).max(dh.abs() / h)`, sides `(w * k).round().max(1.0)`, spanning from the fixed corner toward
    // the drag), :160 (only for a corner).
    // new: `about_centre`.
    if handle[0] == 0 || handle[1] == 0 {
        return resized_rect(rect, handle, by, about_centre);
    }
    let (dx, dy) = (by.x.round(), by.y.round());
    let (w, h) = (rect.width().max(1.0), rect.height().max(1.0));
    let (cx, cy) = ((rect.x0 + rect.x1) / 2.0, (rect.y0 + rect.y1) / 2.0);
    // The fixed point: the opposite corner, or the centre with `about_centre`; and the dragged
    // corner's offset from it.
    let (ax, ay) =
        if about_centre { (cx, cy) } else { (if handle[0] > 0 { rect.x0 } else { rect.x1 }, if handle[1] > 0 { rect.y0 } else { rect.y1 }) };
    let (px, py) = (if handle[0] > 0 { rect.x1 } else { rect.x0 } + dx, if handle[1] > 0 { rect.y1 } else { rect.y0 } + dy);
    let (dw, dh) = (px - ax, py - ay);
    let reach = if about_centre { 2.0 } else { 1.0 };
    let k = (dw.abs() * reach / w).max(dh.abs() * reach / h);
    let (nw, nh) = ((w * k).round().max(1.0), (h * k).round().max(1.0));
    if about_centre {
        let (x0, y0) = ((cx - nw / 2.0).round(), (cy - nh / 2.0).round());
        return Rect::new(x0, y0, x0 + nw, y0 + nh);
    }
    let span = |a: f64, len: f64, toward: f64| if toward < 0.0 { [a - len, a] } else { [a, a + len] };
    let ([x0, x1], [y0, y1]) = (span(ax, nw, dw), span(ay, nh, dh));
    Rect::new(x0, y0, x1, y1)
}
