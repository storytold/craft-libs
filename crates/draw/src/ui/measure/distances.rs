//! [`box_distances`]: the distances between two boxes, as lines and measures for the existing
//! [`SmartGuides`](craft_ui::canvas_controls::SmartGuides). No egui state.

use kurbo::{Point, Rect};

use craft_behavior::Axis;
use craft_ui::canvas_controls::{Measure, SmartLine};

/// The distances from box `a` (the selection's bounds) to box `b` (the hovered layer's, the
/// canvas, or an ⌥-drag's copy), per axis: the gap between facing edges when the boxes are apart
/// on that axis, else the distances between like edges (left to left, right to right). Zero and
/// non-finite distances are dropped. A distance runs through the middle of the boxes' overlap on
/// the other axis, else through `a`'s centre; where it ends beside `b`, a dashed extension
/// ([`SmartLine::extension`]) continues `b`'s edge to it. `label` formats a length in document
/// units along an axis ([`Axis::X`]: a distance along x) in the app's ruler unit.
///
/// ```
/// use craft_draw::ui::measure::box_distances;
/// use craft_behavior::kurbo::Rect;
/// let (lines, measures) = box_distances(Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(30.0, 0.0, 40.0, 10.0), |len, _| format!("{len} px"));
/// // The app shows them with `SmartGuides::new(&lines, &measures)`.
/// # let _ = (lines, measures);
/// ```
///
/// The app decides when they show (Smart Guides on, ⌘ held with no button down over another layer
/// or the empty canvas, an ⌥-drag copy).
pub fn box_distances(a: Rect, b: Rect, label: impl Fn(f64, Axis) -> String) -> (Vec<SmartLine>, Vec<Measure>) {
    // pc: crates/ui-egui/src/tools/move_distance.rs:50 `measure` (each `Measure { horizontal, at,
    // from, to }` becomes a `Measure` with its label; each extension `[x0, y0, x1, y1]` a
    // `SmartLine` with `extension: true`).
    let (mut lines, mut measures) = (Vec::new(), Vec::new());
    let coords = [a.x0, a.y0, a.x1, a.y1, b.x0, b.y0, b.x1, b.y1];
    if !coords.iter().all(|v| v.is_finite()) {
        return (lines, measures);
    }
    let (a, b) = ([a.x0, a.y0, a.x1, a.y1], [b.x0, b.y0, b.x1, b.y1]);
    for horizontal in [true, false] {
        // Spans along the measured axis (`sa`, `sb`) and across it (`oa`, `ob`).
        let (k, o) = if horizontal { (0, 1) } else { (1, 0) };
        let (sa, sb) = ([a[k], a[k + 2]], [b[k], b[k + 2]]);
        let (oa, ob) = ([a[o], a[o + 2]], [b[o], b[o + 2]]);
        let (lo, hi) = (oa[0].max(ob[0]), oa[1].min(ob[1]));
        let at = if lo < hi { (lo + hi) / 2.0 } else { f64::midpoint(oa[0], oa[1]) };
        // Each pair: the two ends, and which of them lies on `b`'s edge.
        let pairs = if sa[1] <= sb[0] {
            vec![(sa[1], sb[0], sb[0])]
        } else if sb[1] <= sa[0] {
            vec![(sb[1], sa[0], sb[1])]
        } else {
            vec![(sa[0].min(sb[0]), sa[0].max(sb[0]), sb[0]), (sa[1].min(sb[1]), sa[1].max(sb[1]), sb[1])]
        };
        // A point on the measured axis (`s`) and across it (`across`).
        let point = |s: f64, across: f64| if horizontal { Point::new(s, across) } else { Point::new(across, s) };
        let axis = if horizontal { Axis::X } else { Axis::Y };
        for (from, to, edge) in pairs {
            if to - from <= 0.0 {
                continue;
            }
            measures.push(Measure::new(point(from, at), point(to, at)).labelled(label(to - from, axis)));
            // The line ends beside `b`: extend `b`'s edge to it.
            if at < ob[0] || at > ob[1] {
                let near = if at < ob[0] { ob[0] } else { ob[1] };
                lines.push(SmartLine::new(point(edge, near), point(edge, at)).dashed());
            }
        }
    }
    (lines, measures)
}
