//! Where guides are and how they move: screen segments, keyboard order and the edits of a vector of guides.

use crate::canvas::{Guide, GuideKey};
use craft_behavior::geometry::across_axis;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{Axis, CanvasView};
use egui::{Pos2, Rect};

/// How far past the canvas edge a guide on the edge may sit: rounding can put it just outside.
const EDGE_SLACK: f32 = 0.5;

/// The `lo..hi` of `rect` across a guide running `along`: the x range for `Axis::Y`.
pub(super) fn across_range(rect: DocRect, along: Axis) -> (f64, f64) {
    match along {
        Axis::X => (rect.y0, rect.y1),
        _ => (rect.x0, rect.x1),
    }
}

/// The document point `length` along a guide at `position`.
fn point(along: Axis, position: f64, length: f64) -> Point {
    match along {
        Axis::X => Point::new(length, position),
        _ => Point::new(position, length),
    }
}

/// Screen points per document unit across a guide running `along`.
pub(super) fn zoom_across(view: &CanvasView, along: Axis) -> f64 {
    match along {
        Axis::X => view.zoom,
        _ => view.zoom_x(),
    }
}

/// The screen ends of a guide at `position` across what the view shows, limited to `span` and to `area`: a rotated
/// view turns the guide with the document. `None` when the guide is out of view.
pub(super) fn segment(view: &CanvasView, along: Axis, position: f64, span: Option<(f64, f64)>, area: Rect) -> Option<[Pos2; 2]> {
    let (lo, hi) = across_range(view.visible(), across_axis(along));
    let (lo, hi) = match span {
        Some((from, to)) => (lo.max(from.min(to)), hi.min(to.max(from))),
        None => (lo, hi),
    };
    if lo > hi {
        return None;
    }
    clip(area.expand(EDGE_SLACK), view.to_screen(point(along, position, lo)), view.to_screen(point(along, position, hi)))
}

/// The part of the segment from `a` to `b` inside `rect`, if any (Liang-Barsky).
fn clip(rect: Rect, a: Pos2, b: Pos2) -> Option<[Pos2; 2]> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, q) in [(-d.x, a.x - rect.min.x), (d.x, rect.max.x - a.x), (-d.y, a.y - rect.min.y), (d.y, rect.max.y - a.y)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some([a + d * t0, a + d * t1])
}

/// The indices of the unlocked guides in keyboard order: vertical guides left to right, then horizontal ones top to
/// bottom.
pub(super) fn keyboard_order(guides: &[Guide]) -> Vec<usize> {
    let rank = |guide: &Guide| match guide.along {
        Axis::X => 1,
        _ => 0,
    };
    let mut order: Vec<usize> = (0..guides.len()).filter(|&at| !guides[at].locked).collect();
    order.sort_by(|&a, &b| rank(&guides[a]).cmp(&rank(&guides[b])).then(guides[a].position.total_cmp(&guides[b].position)));
    order
}

/// `position` kept inside `lo..hi`, whichever way round they are given.
pub(super) fn clamped(position: f64, (lo, hi): (f64, f64)) -> f64 {
    position.clamp(lo.min(hi), hi.max(lo))
}

/// The direction an arrow key moves a guide running `along`: -1, 0 or 1 across it. `direction` is the arrow on
/// screen and the view turns it into the document direction; an arrow that runs along the guide moves nothing.
pub(super) fn nudge_sign(view: &CanvasView, along: Axis, direction: egui::Vec2) -> f64 {
    let document = view.unmap_vec(direction);
    let (across, length) = match along {
        Axis::X => (document.y, document.x),
        _ => (document.x, document.y),
    };
    if across.abs() > length.abs() { across.signum() } else { 0.0 }
}

/// `guides` with the guide `key` at `position`.
pub(super) fn moved(guides: &[Guide], key: GuideKey, position: f64) -> Vec<Guide> {
    guides.iter().cloned().map(|guide| if guide.key == key { Guide { position, ..guide } } else { guide }).collect()
}

/// `guides` without the guide `key`.
pub(super) fn removed(guides: &[Guide], key: GuideKey) -> Vec<Guide> {
    guides.iter().filter(|guide| guide.key != key).cloned().collect()
}

/// `guides` with `guide` in place of the one with its key, or at the end when none has it.
pub(super) fn with_guide(guides: &[Guide], guide: Guide) -> Vec<Guide> {
    let mut all = removed(guides, guide.key);
    all.push(guide);
    all
}

/// The key a new guide takes: one past the greatest in use.
pub(super) fn fresh_key(guides: &[Guide]) -> GuideKey {
    GuideKey::new(guides.iter().map(|guide| guide.key.get() + 1).max().unwrap_or(0))
}

/// The guide that holds the place of `key` once it is gone: the next in `order`, else the one before it.
pub(super) fn after_removal(guides: &[Guide], order: &[usize], key: GuideKey) -> Option<GuideKey> {
    let at = order.iter().position(|&index| guides[index].key == key)?;
    order.get(at + 1).or_else(|| at.checked_sub(1).and_then(|before| order.get(before))).map(|&index| guides[index].key)
}
