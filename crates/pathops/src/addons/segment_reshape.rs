//! Reshaping a segment by dragging a point on it, shared by the add-on tools that bend segments
//! directly. The grabbed point follows the pointer; how the two handles share the move depends on
//! where the segment was grabbed (the nearer end's handle moves more) and on the [`ReshapeMode`].
//! A missing handle grows out of its anchor, and a smooth anchor's other handle turns with the one
//! moved, so the path stays smooth there.

use craft_geom::SubPath;
use kurbo::Vec2;

/// The range the grab position `t` is held to: right at an end, the handle beside it would have
/// to grow without bound to move the point.
pub const T_RANGE: (f64, f64) = (0.05, 0.95);

/// How [`reshape_segment`] moves the handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReshapeMode {
    /// Both handles move in the direction of the drag, the one nearer the grab more.
    Free,
    /// Each handle keeps its direction and only changes length (a missing one points along the
    /// segment's chord). A segment whose two directions are parallel can't bend that way, and is
    /// reshaped [`ReshapeMode::Free`].
    KeepAngles,
    /// Both handles stand at right angles to the chord, on one side and at one length.
    Symmetric,
}

impl ReshapeMode {
    pub const ALL: [Self; 3] = [Self::Free, Self::KeepAngles, Self::Symmetric];

    pub fn id(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::KeepAngles => "keepAngles",
            Self::Symmetric => "symmetric",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == s)
    }
}

fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l > 1e-9 && l.is_finite()).then(|| v / l)
}

/// Bend segment `seg` of `sp` so its point at `t` (held to [`T_RANGE`]) moves by `dv`, the way
/// `mode` says. False, leaving `sp` as it was, when there is no such segment, an input isn't
/// finite or the result wouldn't be.
pub fn reshape_segment(sp: &mut SubPath, seg: usize, t: f64, dv: Vec2, mode: ReshapeMode) -> bool {
    if seg >= sp.segment_count() || !t.is_finite() || !dv.x.is_finite() || !dv.y.is_finite() {
        return false;
    }
    let t = t.clamp(T_RANGE.0, T_RANGE.1);
    let mut out = sp.clone();
    let bent = match mode {
        ReshapeMode::Symmetric => out.bend_segment_perpendicular(seg, t, dv),
        ReshapeMode::KeepAngles => keep_angles(&mut out, seg, t, dv) || free(&mut out, seg, t, dv),
        ReshapeMode::Free => free(&mut out, seg, t, dv),
    };
    let finite = out.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite());
    if !bent || !finite {
        return false;
    }
    *sp = out;
    true
}

/// The weights the segment's two inner control points have at `t`.
fn weights(t: f64) -> (f64, f64) {
    let s = 1.0 - t;
    (3.0 * s * s * t, 3.0 * s * t * t)
}

/// Move the two handles by shares of `dv` weighted by the grab position: the share each gets is in
/// proportion to how near its end the grab is, scaled so the point at `t` lands exactly `dv` away
/// (the smallest move of the two that does it).
fn free(sp: &mut SubPath, seg: usize, t: f64, dv: Vec2) -> bool {
    let n = sp.anchors.len();
    let (w1, w2) = weights(t);
    let (a, b) = (1.0 - t, t);
    let k = 1.0 / (w1 * a + w2 * b);
    if let Some(p) = sp.anchors.get_mut(seg % n) {
        p.set_handle(true, p.h_out + dv * (k * a), false);
    }
    if let Some(q) = sp.anchors.get_mut((seg + 1) % n) {
        q.set_handle(false, q.h_in + dv * (k * b), false);
    }
    true
}

/// Lengthen or shorten the two handles along their own directions so the point at `t` moves by
/// `dv`; a handle that would turn back on itself is retracted. False when the directions are
/// parallel (no such move exists).
fn keep_angles(sp: &mut SubPath, seg: usize, t: f64, dv: Vec2) -> bool {
    let n = sp.anchors.len();
    let c = sp.segment(seg);
    let (Some(u1), Some(u2)) = (unit(c.p1 - c.p0).or_else(|| unit(c.p3 - c.p0)), unit(c.p2 - c.p3).or_else(|| unit(c.p0 - c.p3))) else {
        return false;
    };
    let det = u1.cross(u2);
    if det.abs() < 1e-6 {
        return false;
    }
    let (w1, w2) = weights(t);
    // Cramer's rule for x·u1 + y·u2 = dv, then each handle's change of length.
    let (x, y) = (dv.cross(u2) / det, u1.cross(dv) / det);
    let len1 = ((c.p1 - c.p0).dot(u1) + x / w1).max(0.0);
    let len2 = ((c.p2 - c.p3).dot(u2) + y / w2).max(0.0);
    if let Some(p) = sp.anchors.get_mut(seg % n) {
        p.set_handle(true, c.p0 + u1 * len1, false);
    }
    if let Some(q) = sp.anchors.get_mut((seg + 1) % n) {
        q.set_handle(false, c.p3 + u2 * len2, false);
    }
    true
}

#[cfg(test)]
mod tests {
    use craft_geom::{Anchor, AnchorKind, SubPath};
    use kurbo::{ParamCurve, Point, Vec2};

    use super::*;

    fn pt(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn line() -> SubPath {
        SubPath::polyline(&[pt(0.0, 0.0), pt(100.0, 0.0)], false)
    }

    #[test]
    fn the_grabbed_point_follows_the_drag_and_the_nearer_handle_moves_more() {
        for t in [0.2, 0.5, 0.8] {
            let mut sp = line();
            let before = sp.segment(0).eval(t);
            assert!(reshape_segment(&mut sp, 0, t, Vec2::new(0.0, 30.0), ReshapeMode::Free));
            let after = sp.segment(0).eval(t);
            assert!((after - before - Vec2::new(0.0, 30.0)).hypot() < 1e-9, "t {t}: {after:?}");
            // The handles grew out of the anchors, the one nearer the grab the longer.
            let (h1, h2) = ((sp.anchors[0].h_out - sp.anchors[0].p).hypot(), (sp.anchors[1].h_in - sp.anchors[1].p).hypot());
            assert!(h1 > 0.0 && h2 > 0.0);
            assert_eq!(h1 > h2, t < 0.5, "t {t}: {h1} {h2}");
        }
    }

    #[test]
    fn a_smooth_neighbour_stays_smooth() {
        let mut sp =
            SubPath::new(vec![Anchor::corner(pt(0.0, 0.0)), Anchor::smooth(pt(100.0, 0.0), pt(130.0, 0.0)), Anchor::corner(pt(200.0, 0.0))], false);
        assert!(reshape_segment(&mut sp, 0, 0.7, Vec2::new(0.0, -20.0), ReshapeMode::Free));
        let a = sp.anchors[1];
        assert_eq!(a.kind, AnchorKind::Smooth);
        assert!((a.h_out - a.p).cross(a.p - a.h_in).abs() < 1e-9, "the handles stay in line");
        assert!(((a.h_out - a.p).hypot() - 30.0).abs() < 1e-9, "the other handle keeps its length");
    }

    #[test]
    fn keep_angles_only_changes_lengths() {
        let mut sp = SubPath::new(
            vec![
                Anchor::with_handles(pt(0.0, 0.0), pt(0.0, 0.0), pt(20.0, -20.0)),
                Anchor::with_handles(pt(100.0, 0.0), pt(80.0, -20.0), pt(100.0, 0.0)),
            ],
            false,
        );
        let before = sp.segment(0).eval(0.5);
        assert!(reshape_segment(&mut sp, 0, 0.5, Vec2::new(0.0, -10.0), ReshapeMode::KeepAngles));
        let (a, b) = (sp.anchors[0], sp.anchors[1]);
        assert!((a.h_out - a.p).cross(Vec2::new(1.0, -1.0)).abs() < 1e-9 && (b.h_in - b.p).cross(Vec2::new(-1.0, -1.0)).abs() < 1e-9);
        assert!((sp.segment(0).eval(0.5) - before - Vec2::new(0.0, -10.0)).hypot() < 1e-9);
        // A straight segment has parallel directions: it bends freely instead.
        let mut sp = line();
        assert!(reshape_segment(&mut sp, 0, 0.5, Vec2::new(0.0, 10.0), ReshapeMode::KeepAngles));
        assert!((sp.segment(0).eval(0.5) - pt(50.0, 10.0)).hypot() < 1e-9);
    }

    #[test]
    fn symmetric_makes_the_handles_square_to_the_chord() {
        let mut sp = line();
        assert!(reshape_segment(&mut sp, 0, 0.3, Vec2::new(5.0, 20.0), ReshapeMode::Symmetric));
        let (a, b) = (sp.anchors[0], sp.anchors[1]);
        assert!((a.h_out.x - a.p.x).abs() < 1e-9 && (b.h_in.x - b.p.x).abs() < 1e-9);
        assert!(((a.h_out - a.p).hypot() - (b.h_in - b.p).hypot()).abs() < 1e-9);
    }

    #[test]
    fn bad_input_changes_nothing() {
        let mut sp = line();
        for (seg, t, dv) in [
            (1, 0.5, Vec2::new(0.0, 1.0)),
            (0, f64::NAN, Vec2::new(0.0, 1.0)),
            (0, 0.5, Vec2::new(f64::INFINITY, 0.0)),
            (0, 0.05, Vec2::new(1e308, 1e308)),
        ] {
            assert!(!reshape_segment(&mut sp, seg, t, dv, ReshapeMode::Free), "{seg} {t} {dv:?}");
            assert_eq!(sp, line());
        }
        assert!(!reshape_segment(&mut SubPath::default(), 0, 0.5, Vec2::new(1.0, 1.0), ReshapeMode::KeepAngles));
        // A grab at an end is held inside the range, so the handles stay finite.
        assert!(reshape_segment(&mut sp, 0, 0.0, Vec2::new(0.0, 1.0), ReshapeMode::Free));
        assert_eq!(ReshapeMode::parse("keepAngles"), Some(ReshapeMode::KeepAngles));
        assert_eq!(ReshapeMode::parse("sideways"), None);
    }
}
