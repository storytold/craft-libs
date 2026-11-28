//! The transform box: corner, edge, turn and distort drags.

use craft_behavior::geometry::{BoxPart, BoxRules, BoxState, ScaleRule, drag};
use craft_keymap::Mods;
use kurbo::{Point, Rect, Vec2};

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// 100 by 50 at the origin, the reference point at its centre.
fn session() -> BoxState {
    BoxState::from_rect(Rect::new(0.0, 0.0, 100.0, 50.0))
}

#[test]
fn corner_drag_preserves_the_offset_of_the_press() {
    let start = session();
    let press = p(96.0, 47.0);
    let rules = BoxRules::default();
    let back = drag(&start, &start, BoxPart::Corner(2), press, press, Mods::default(), &rules);
    assert_eq!(back, start, "returning to the press must restore the box");
    let moved = drag(&start, &start, BoxPart::Corner(2), press, press + Vec2::new(20.0, 10.0), Mods::SHIFT, &rules);
    assert!(near(moved.quad[2], start.quad[2] + Vec2::new(20.0, 10.0)), "the corner follows the movement, keeping the press offset");
    assert!(near(moved.quad[0], start.quad[0]), "the opposite corner stays fixed");
}

fn dragged(from: Point, to: Point, mods: Mods, rules: &BoxRules) -> BoxState {
    let start = session();
    drag(&start, &start, start.part_at(from, 8.0), from, to, mods, rules)
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

fn corners_are(s: &BoxState, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    let want = BoxState::from_rect(Rect::new(x0, y0, x1, y1)).quad;
    s.quad.iter().zip(&want).all(|(a, b)| near(*a, *b))
}

fn turns(q: &[Point; 4]) -> [f64; 4] {
    std::array::from_fn(|k| (q[(k + 1) % 4] - q[k]).cross(q[(k + 2) % 4] - q[(k + 1) % 4]))
}

#[test]
fn proportional_corner_uses_the_larger_relative_drag_even_when_shrinking() {
    let rules = BoxRules::default().with_scale(ScaleRule::LargerAxis);
    let wider = dragged(p(100.0, 50.0), p(200.0, 60.0), Mods::NONE, &rules);
    assert!(corners_are(&wider, 0.0, 0.0, 200.0, 100.0));
    let smaller = dragged(p(100.0, 50.0), p(75.0, 50.0), Mods::NONE, &rules);
    assert!(corners_are(&smaller, 0.0, 0.0, 75.0, 37.5));
}

#[test]
fn a_proportional_corner_follows_the_diagonal_and_flips_past_the_anchor() {
    let rules = BoxRules::default();
    // (200, 60) projects on the diagonal from (0, 0) through (100, 50) at 1.84 of it.
    let kept = dragged(p(100.0, 50.0), p(200.0, 60.0), Mods::NONE, &rules);
    assert!(corners_are(&kept, 0.0, 0.0, 184.0, 92.0), "{:?}", kept.quad);
    let flipped = dragged(p(100.0, 50.0), p(-100.0, -50.0), Mods::NONE, &rules);
    assert!(corners_are(&flipped, 0.0, 0.0, -100.0, -50.0), "{:?}", flipped.quad);
    let freed = dragged(p(100.0, 50.0), p(200.0, 60.0), Mods::SHIFT, &rules);
    assert!(corners_are(&freed, 0.0, 0.0, 200.0, 60.0), "{:?}", freed.quad);
    // A distorted box: the corner lands on the pointer's projection on the anchor–corner line.
    let start = BoxState::new([p(0.0, 0.0), p(100.0, 0.0), p(120.0, 80.0), p(0.0, 50.0)], p(50.0, 30.0));
    let to = p(150.0, 120.0);
    let got = drag(&start, &start, BoxPart::Corner(2), p(120.0, 80.0), to, Mods::NONE, &rules);
    let d = Vec2::new(120.0, 80.0);
    let want = Point::ORIGIN + d * (to.to_vec2().dot(d) / d.hypot2());
    assert!(near(got.quad[2], want), "{:?} vs {want:?}", got.quad);
    assert!(near(got.quad[0], p(0.0, 0.0)), "{:?}", got.quad);
}

/// The angle of a box turned 25.59° after an outside drag turning `by` degrees about its centre.
fn turned(by: f64, mods: Mods) -> f64 {
    let start = session().turned_about_pivot(25.59f64.to_radians());
    let c = start.pivot;
    let at = |deg: f64| c + Vec2::from_angle(deg.to_radians()) * 200.0;
    let from = at(0.0);
    assert_eq!(start.part_at(from, 8.0), BoxPart::Outside);
    drag(&start, &start, BoxPart::Outside, from, at(by), mods, &BoxRules::default()).angle()
}

#[test]
fn shift_snaps_the_turn_not_the_absolute_angle() {
    let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "{a} != {b}");
    close(turned(20.0, Mods::SHIFT), 40.59);
    close(turned(-20.0, Mods::SHIFT), 10.59);
    close(turned(7.0, Mods::SHIFT), 25.59);
    close(turned(8.0, Mods::SHIFT), 40.59);
    close(turned(20.0, Mods::NONE), 45.59);
}

#[test]
fn a_distort_corner_stops_before_the_box_folds() {
    let rules = BoxRules::default();
    let start = session();
    // The top left far past the bottom right.
    let q = drag(&start, &start, BoxPart::Corner(0), p(0.0, 0.0), p(300.0, 200.0), Mods::CMD, &rules).quad;
    let sign = turns(&start.quad)[0].signum();
    assert!(turns(&q).iter().all(|t| t * sign > 0.0), "still convex: {q:?}");
    assert_eq!(q[1..], start.quad[1..], "the other corners stay put");
    // As far as it could along the drag: up to the diagonal from (100, 0) to (0, 50).
    assert!((q[0].x * 2.0 - q[0].y * 3.0).abs() < 1e-6, "{q:?}");
    assert!((q[0] - p(300.0 / 7.0, 200.0 / 7.0)).hypot() < 0.5, "{q:?}");
    // A drag that keeps the box convex is not held back.
    let q = drag(&start, &start, BoxPart::Corner(0), p(0.0, 0.0), p(2.0, 4.0), Mods::CMD, &rules).quad;
    assert!(near(q[0], p(2.0, 4.0)), "{q:?}");
    // A flat box has no fold to stop at: the corner goes where it is dragged.
    let flat = BoxState::new([p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0), p(3.0, 0.0)], p(1.5, 0.0));
    let q = drag(&flat, &flat, BoxPart::Corner(0), p(0.0, 0.0), p(5.0, 5.0), Mods::CMD, &rules).quad;
    assert_eq!(q, [p(5.0, 5.0), p(1.0, 0.0), p(2.0, 0.0), p(3.0, 0.0)]);
}

#[test]
fn a_corners_box_keeps_a_corner_proportional_until_shift_and_stretches_an_edge_on_one_axis() {
    let linked = BoxRules::default().with_scale(ScaleRule::Corners);
    let unlinked = linked.with_aspect_linked(false);
    for rules in [&linked, &unlinked] {
        // A plain corner keeps the proportions along the diagonal, whatever the aspect link says.
        let kept = dragged(p(100.0, 50.0), p(200.0, 60.0), Mods::NONE, rules);
        assert!(corners_are(&kept, 0.0, 0.0, 184.0, 92.0), "{:?}", kept.quad);
        // ⇧ frees them.
        let freed = dragged(p(100.0, 50.0), p(200.0, 60.0), Mods::SHIFT, rules);
        assert!(corners_are(&freed, 0.0, 0.0, 200.0, 60.0), "{:?}", freed.quad);
        // The right edge's middle stretches only x, with ⇧ or without.
        for mods in [Mods::NONE, Mods::SHIFT] {
            let edge = dragged(p(100.0, 25.0), p(150.0, 40.0), mods, rules);
            assert!(corners_are(&edge, 0.0, 0.0, 150.0, 50.0), "{mods:?}: {:?}", edge.quad);
        }
    }
    // A ⌘ corner distorts as it does under the default rule: the gesture is left alone.
    let distorted = dragged(p(100.0, 50.0), p(200.0, 60.0), Mods::CMD, &linked);
    assert_eq!(distorted, dragged(p(100.0, 50.0), p(200.0, 60.0), Mods::CMD, &BoxRules::default()));
    assert!(near(distorted.quad[2], p(200.0, 60.0)) && near(distorted.quad[0], p(0.0, 0.0)), "{:?}", distorted.quad);
}

#[test]
fn a_shape_box_scales_corners_by_the_diagonal_and_edges_on_one_axis() {
    // The reference measurement: the bounds 100,98–723,451, the bottom-right handle dragged by
    // (−100, −60), give about 100,98–621,394.
    let start = BoxState::from_rect(Rect::new(100.0, 98.0, 723.0, 451.0));
    let shaped = |from: Point, to: Point, mods: Mods, rules: &BoxRules| drag(&start, &start, start.part_at(from, 8.0), from, to, mods, rules);
    let linked = BoxRules::default().with_scale(ScaleRule::Shape);
    let unlinked = linked.with_aspect_linked(false);
    let s = 1.0 - 83_480.0 / 512_738.0;
    for (mods, rules) in [(Mods::NONE, &linked), (Mods::SHIFT, &linked), (Mods::NONE, &unlinked)] {
        let corner = shaped(p(723.0, 451.0), p(623.0, 391.0), mods, rules);
        assert!(corners_are(&corner, 100.0, 98.0, 100.0 + 623.0 * s, 98.0 + 353.0 * s), "{mods:?}: {:?}", corner.quad);
        assert!((corner.quad[2] - p(621.0, 394.0)).hypot() < 1.0, "{:?}", corner.quad);
        // The right edge's middle, dragged 100 left: only x scales, about the left edge.
        let edge = shaped(p(723.0, 274.5), p(623.0, 274.5), mods, rules);
        assert!(corners_are(&edge, 100.0, 98.0, 623.0, 451.0), "{mods:?}: {:?}", edge.quad);
    }
}
