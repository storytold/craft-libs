//! Smart Pen geometry: making a segment straight or curved (the curve leaving its ends tangent to
//! what comes before them), toggling an anchor between smooth and corner, and deleting an anchor
//! without refitting the handles beside it.

use craft_geom::{AnchorKind, SubPath};
use kurbo::Vec2;

use super::pathtools::{auto_smooth, make_corner, neighbours};

fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l > 1e-9 && l.is_finite()).then(|| v / l)
}

/// Give anchor `i` the handle facing its next anchor (`out`) or its previous one, when it has none,
/// so the segment between them curves; `ratio` × the distance to that anchor long. The handle is
/// in line with the anchor's other handle (the anchor becomes smooth); else, after a straight
/// segment, in line with that segment (a straight-to-curved joint, the curve leaving tangent);
/// else the anchor is smoothed by the auto-smooth rule. False when nothing changes.
pub fn leave_handle(sp: &mut SubPath, i: usize, out: bool, ratio: f64) -> bool {
    let (prev, next) = neighbours(sp, i);
    let (Some(facing), back) = (if out { (next, prev) } else { (prev, next) }) else { return false };
    let at = |j: usize| sp.anchors.get(j).copied();
    let (Some(a), Some(f)) = (at(i), at(facing)) else { return false };
    if (out && a.has_out()) || (!out && a.has_in()) {
        return false;
    }
    let len = a.p.distance(f.p) * ratio;
    let opposite = if out { a.h_in } else { a.h_out };
    // The back segment is straight when the back anchor's handle toward `i` is retracted too.
    let straight_back = back.and_then(at).filter(|b| if out { !b.has_out() } else { !b.has_in() });
    let (dir, kind) = match (unit(a.p - opposite), straight_back) {
        (Some(d), _) => (d, AnchorKind::Smooth),
        (None, Some(b)) => match unit(a.p - b.p) {
            Some(d) => (d, a.kind),
            None => return false,
        },
        (None, None) => return auto_smooth(sp, i, ratio),
    };
    let Some(a) = sp.anchors.get_mut(i) else { return false };
    let h = a.p + dir * len;
    if out {
        a.h_out = h;
    } else {
        a.h_in = h;
    }
    a.kind = kind;
    true
}

/// Make segment `seg` curved: each of its ends without a handle on it gets one ([`leave_handle`]).
/// False when nothing changes.
pub fn curve_segment(sp: &mut SubPath, seg: usize, ratio: f64) -> bool {
    let n = sp.anchors.len();
    if seg >= sp.segment_count() {
        return false;
    }
    let from = leave_handle(sp, seg % n, true, ratio);
    let to = leave_handle(sp, (seg + 1) % n, false, ratio);
    from || to
}

/// Make segment `seg` straight: the handles its two ends have on it are retracted, and an end left
/// with one handle is a corner. False when it was straight already.
pub fn straighten_segment(sp: &mut SubPath, seg: usize) -> bool {
    let n = sp.anchors.len();
    if seg >= sp.segment_count() || sp.segment_is_line(seg) {
        return false;
    }
    if let Some(a) = sp.anchors.get_mut(seg % n) {
        a.h_out = a.p;
        a.kind = AnchorKind::Corner;
    }
    if let Some(b) = sp.anchors.get_mut((seg + 1) % n) {
        b.h_in = b.p;
        b.kind = AnchorKind::Corner;
    }
    true
}

/// Toggle anchor `i` between smooth and corner: a smooth anchor becomes a corner with its handles
/// where they are (nothing moves); any other is smoothed by the auto-smooth rule at `ratio`.
/// False when nothing changes.
pub fn toggle_smooth(sp: &mut SubPath, i: usize, ratio: f64) -> bool {
    let Some(a) = sp.anchors.get(i) else { return false };
    if a.kind == AnchorKind::Smooth && a.has_in() && a.has_out() { make_corner(sp, i, false) } else { auto_smooth(sp, i, ratio) }
}

/// Delete anchor `i`, leaving the handles of its neighbours as they are (the segment between them
/// takes their handles as they stand). False, leaving `sp` alone, when there is no such anchor or
/// the subpath would keep fewer than two anchors (three when closed).
pub fn delete_anchor(sp: &mut SubPath, i: usize) -> bool {
    let min = if sp.closed { 3 } else { 2 };
    if i >= sp.anchors.len() || sp.anchors.len() <= min {
        return false;
    }
    sp.anchors.remove(i);
    if !sp.closed {
        // A new end has nothing beyond it.
        if let Some(a) = sp.anchors.first_mut() {
            a.h_in = a.p;
        }
        if let Some(a) = sp.anchors.last_mut() {
            a.h_out = a.p;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use craft_geom::{Anchor, AnchorKind, SubPath};
    use kurbo::Point;

    use super::*;
    use crate::addons::pathtools::SMOOTH_RATIO;

    fn pt(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn parallel(a: Vec2, b: Vec2) -> bool {
        a.cross(b).abs() < 1e-9 && a.dot(b) > 0.0
    }

    #[test]
    fn a_curve_after_a_straight_segment_leaves_tangent_to_it() {
        // (0,0) → (100,0) straight, then a curve on to (150,50).
        let mut sp = SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 0.0), pt(150.0, 50.0)], false);
        assert!(leave_handle(&mut sp, 1, true, SMOOTH_RATIO));
        let a = sp.anchors[1];
        assert!(parallel(a.h_out - a.p, Vec2::new(1.0, 0.0)), "{:?}", a.h_out);
        assert!(((a.h_out - a.p).hypot() - 50f64.hypot(50.0) * SMOOTH_RATIO).abs() < 1e-9);
        assert!(!a.has_in(), "the straight segment stays straight");
        assert!(sp.segment_is_line(0) && !sp.segment_is_line(1));
        assert!(!leave_handle(&mut sp, 1, true, SMOOTH_RATIO), "it has one already");
    }

    #[test]
    fn a_lone_handle_is_continued_and_corners_between_curves_are_smoothed() {
        let mut sp = SubPath::new(
            vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(30.0, -10.0), pt(50.0, 0.0)), Anchor::corner(pt(100.0, 0.0))],
            false,
        );
        assert!(leave_handle(&mut sp, 1, true, SMOOTH_RATIO));
        let a = sp.anchors[1];
        assert_eq!(a.kind, AnchorKind::Smooth);
        assert!(parallel(a.h_out - a.p, a.p - a.h_in));
        // A corner without handles after a curved segment: smoothed by the auto-smooth rule.
        let mut sp = SubPath::new(
            vec![Anchor::with_handles(pt(0.0, 0.0), pt(0.0, 0.0), pt(20.0, -30.0)), Anchor::corner(pt(50.0, 0.0)), Anchor::corner(pt(100.0, 0.0))],
            false,
        );
        assert!(leave_handle(&mut sp, 1, true, SMOOTH_RATIO));
        assert!(sp.anchors[1].has_in() && sp.anchors[1].has_out() && sp.anchors[1].kind == AnchorKind::Smooth);
    }

    #[test]
    fn segments_toggle_between_straight_and_curved() {
        let mut sp = SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 0.0), pt(100.0, 100.0), pt(0.0, 100.0)], false);
        assert!(curve_segment(&mut sp, 1, SMOOTH_RATIO));
        assert!(!sp.segment_is_line(1) && sp.segment_is_line(0) && sp.segment_is_line(2));
        assert!(!curve_segment(&mut sp, 1, SMOOTH_RATIO), "curved already");
        assert!(straighten_segment(&mut sp, 1));
        assert!(sp.segment_is_line(1));
        assert!(!straighten_segment(&mut sp, 1));
        assert!(!curve_segment(&mut sp, 3, SMOOTH_RATIO) && !straighten_segment(&mut sp, 3), "no such segment");
    }

    #[test]
    fn toggling_a_point_moves_nothing_one_way_and_smooths_the_other() {
        let mut sp = SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 0.0), pt(100.0, 100.0)], false);
        assert!(toggle_smooth(&mut sp, 1, SMOOTH_RATIO));
        let smooth = sp.anchors[1];
        assert_eq!(smooth.kind, AnchorKind::Smooth);
        assert!(toggle_smooth(&mut sp, 1, SMOOTH_RATIO));
        let corner = sp.anchors[1];
        assert_eq!((corner.kind, corner.h_in, corner.h_out), (AnchorKind::Corner, smooth.h_in, smooth.h_out));
        assert!(!toggle_smooth(&mut sp, 9, SMOOTH_RATIO));
    }

    #[test]
    fn delete_keeps_the_neighbours_handles() {
        let mut sp = SubPath::new(
            vec![Anchor::smooth(pt(0.0, 0.0), pt(10.0, 10.0)), Anchor::corner(pt(50.0, 0.0)), Anchor::smooth(pt(100.0, 0.0), pt(110.0, -10.0))],
            false,
        );
        assert!(delete_anchor(&mut sp, 1));
        assert_eq!(sp.anchors.len(), 2);
        assert_eq!((sp.anchors[0].h_out, sp.anchors[1].h_in), (pt(10.0, 10.0), pt(90.0, 10.0)));
        assert_eq!((sp.anchors[0].h_in, sp.anchors[1].h_out), (pt(0.0, 0.0), pt(100.0, 0.0)), "the ends have nothing beyond them");
        assert!(!delete_anchor(&mut sp, 0), "two anchors stay");
        let mut tri = SubPath::polyline(&[pt(0.0, 0.0), pt(1.0, 0.0), pt(0.0, 1.0)], true);
        assert!(!delete_anchor(&mut tri, 1) && !delete_anchor(&mut tri, 7));
    }
}
