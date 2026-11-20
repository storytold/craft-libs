//! The transform box: corner, edge, turn and distort drags.

use craft_behavior::geometry::{BoxPart, BoxRules, BoxState, ScaleRule, TransformMode, drag};
use craft_keymap::Mods;
use kurbo::{Point, Rect, Vec2};

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

/// 100 by 50 at the origin, the reference point at its centre.
fn session() -> BoxState {
    BoxState::from_rect(Rect::new(0.0, 0.0, 100.0, 50.0))
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

/// A box after `quad` with an edge dragged from `from` to `to`, no modifiers.
fn edge_dragged(quad: [Point; 4], edge: u8, from: Point, to: Point) -> BoxState {
    let start = BoxState::new(quad, quad[0].midpoint(quad[2]));
    drag(&start, &start, BoxPart::Edge(edge), from, to, Mods::NONE, &BoxRules::default())
}

fn within(got: &BoxState, want: [(f64, f64); 4], tol: f64) -> bool {
    got.quad.iter().zip(want).all(|(a, (x, y))| (a.x - x).abs() <= tol && (a.y - y).abs() <= tol)
}

/// Measured on the reference application's results (a 302 × 202 box skewed 40 px along its top edge, then an edge
/// dragged): the skew's offset stays 40 px while the box scales, and the opposite edge's middle holds.
#[test]
fn an_edge_drag_on_a_skewed_box_keeps_the_skew_offset() {
    let skewed = [p(190.0, 125.0), p(492.0, 125.0), p(452.0, 327.0), p(150.0, 327.0)];
    let cases = [
        (1, p(472.0, 226.0), p(522.0, 226.0), [(190.0, 108.278), (542.0, 108.278), (502.0, 343.722), (150.0, 343.722)]),
        (3, p(170.0, 226.0), p(130.0, 226.0), [(150.0, 111.623), (492.0, 111.623), (452.0, 340.377), (110.0, 340.377)]),
        (2, p(301.0, 327.0), p(301.0, 367.0), [(160.099, 125.0), (521.901, 125.0), (481.901, 367.0), (120.099, 367.0)]),
        (0, p(341.0, 125.0), p(341.0, 95.0), [(167.574, 95.0), (514.426, 95.0), (474.426, 327.0), (127.574, 327.0)]),
    ];
    for (edge, from, to, want) in cases {
        let got = edge_dragged(skewed, edge, from, to);
        assert!(within(&got, want, 2e-3), "edge {edge}: {:?}", got.quad);
    }
    // Skewed along the other axis (the right edge 30 px down): the same rule in the box's own axes.
    let leaning = [p(150.0, 125.0), p(452.0, 155.0), p(452.0, 357.0), p(150.0, 327.0)];
    let got = edge_dragged(leaning, 1, p(452.0, 256.0), p(502.0, 256.0));
    assert!(within(&got, [(151.629, 108.603), (503.14, 143.522), (499.883, 378.315), (148.371, 343.397)], 2e-3), "{:?}", got.quad);
    let got = edge_dragged(leaning, 2, p(301.0, 342.0), p(301.0, 382.0));
    assert!(within(&got, [(120.099, 122.03), (481.901, 157.97), (477.966, 399.579), (116.164, 363.639)], 2e-3), "{:?}", got.quad);
}

/// A box in perspective, its top or bottom edge dragged: the sides keep their insets along the top edge (measured on
/// the reference's results, as above).
#[test]
fn a_top_or_bottom_edge_drag_on_a_box_in_perspective_keeps_the_insets() {
    let narrow_top = [p(210.0, 125.0), p(392.0, 125.0), p(452.0, 327.0), p(150.0, 327.0)];
    let got = edge_dragged(narrow_top, 2, p(301.0, 327.0), p(301.0, 367.0));
    assert!(within(&got, [(191.98, 125.0), (410.02, 125.0), (470.02, 367.0), (131.98, 367.0)], 1e-2), "{:?}", got.quad);
    let got = edge_dragged(narrow_top, 0, p(301.0, 125.0), p(301.0, 95.0));
    assert!(within(&got, [(196.49, 95.0), (405.51, 95.0), (465.51, 327.0), (136.49, 327.0)], 1e-2), "{:?}", got.quad);
    let narrow_bottom = [p(150.0, 125.0), p(452.0, 125.0), p(412.0, 327.0), p(190.0, 327.0)];
    let got = edge_dragged(narrow_bottom, 2, p(301.0, 327.0), p(301.0, 367.0));
    assert!(within(&got, [(120.1, 125.0), (481.9, 125.0), (441.9, 367.0), (160.1, 367.0)], 1e-2), "{:?}", got.quad);
}

/// A box in perspective, its left or right edge dragged (measured on the reference's results, as above): with parallel
/// sides the right side keeps its offset from the left one; with converging sides the dragged side turns onto the line
/// from their meeting point through the moved handle.
#[test]
fn a_side_drag_on_a_box_in_perspective_turns_the_side_toward_the_vanishing_point() {
    let cases = [
        // Parallel sides, the top and bottom converging.
        (
            [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)],
            1,
            (452.0, 226.0),
            (502.0, 226.0),
            [(152.14, 108.85), (503.28, 155.36), (499.0, 309.66), (147.86, 343.15)],
            0.01,
        ),
        (
            [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)],
            3,
            (150.0, 226.0),
            (110.0, 226.0),
            [(112.4, 106.87), (453.71, 152.08), (450.29, 299.92), (108.98, 334.71)],
            0.01,
        ),
        // Converging sides: a narrow top, a narrow bottom, a lopsided top and a skewed one.
        (
            [(180.0, 125.0), (422.0, 125.0), (452.0, 327.0), (150.0, 327.0)],
            1,
            (437.0, 226.0),
            (487.0, 226.0),
            [(180.0, 104.48), (462.39, 104.48), (511.87, 348.16), (150.0, 348.16)],
            0.4,
        ),
        (
            [(150.0, 125.0), (452.0, 125.0), (412.0, 327.0), (190.0, 327.0)],
            3,
            (170.0, 226.0),
            (130.0, 226.0),
            [(100.34, 111.43), (452.0, 111.43), (412.0, 340.14), (159.44, 340.14)],
            0.3,
        ),
        (
            [(190.0, 125.0), (452.0, 125.0), (452.0, 327.0), (150.0, 327.0)],
            1,
            (452.0, 226.0),
            (502.0, 226.0),
            [(190.0, 105.86), (497.79, 105.86), (506.23, 346.39), (150.0, 346.39)],
            0.2,
        ),
        (
            [(200.0, 125.0), (472.0, 125.0), (452.0, 327.0), (150.0, 327.0)],
            3,
            (175.0, 226.0),
            (135.0, 226.0),
            [(166.0, 110.29), (472.0, 110.29), (452.0, 341.99), (103.85, 341.99)],
            0.2,
        ),
    ];
    for (quad, edge, from, to, want, tol) in cases {
        let got = edge_dragged(quad.map(|(x, y)| p(x, y)), edge, p(from.0, from.1), p(to.0, to.1));
        assert!(within(&got, want, tol), "edge {edge} of {quad:?}: {:?}", got.quad);
    }
}

/// A box whose top and bottom edges converge, its top or bottom edge dragged (measured on the reference's results): the
/// dragged edge turns onto the line from where they meet through the moved handle.
#[test]
fn a_top_or_bottom_drag_on_converging_edges_turns_the_edge_toward_the_vanishing_point() {
    let one_corner_down = [(150.0, 125.0), (452.0, 165.0), (452.0, 327.0), (150.0, 327.0)];
    let lopsided = [(180.0, 145.0), (452.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let cases = [
        (one_corner_down, 0, (301.0, 145.0), (301.0, 115.0), [(131.18, 88.7), (477.81, 142.2), (474.43, 329.97), (127.57, 324.03)], 0.5),
        (lopsided, 0, (316.0, 135.0), (316.0, 105.0), [(155.07, 118.66), (472.66, 91.65), (474.69, 325.33), (127.31, 328.67)], 0.25),
        (lopsided, 2, (301.0, 327.0), (301.0, 367.0), [(149.74, 147.22), (482.26, 122.78), (485.88, 369.57), (123.94, 364.03)], 0.75),
    ];
    for (quad, edge, from, to, want, tol) in cases {
        let got = edge_dragged(quad.map(|(x, y)| p(x, y)), edge, p(from.0, from.1), p(to.0, to.1));
        assert!(within(&got, want, tol), "edge {edge} of {quad:?}: {:?}", got.quad);
    }
}

/// The dragged edge's line is exact where its corners are not yet: on the reference's results for long drags on a box in
/// strong perspective, both of its corners lie on the line through the vanishing point we give the edge.
#[test]
fn a_dragged_edge_in_perspective_lies_on_the_reference_line() {
    let parallel_sides = [p(150.0, 125.0), p(452.0, 165.0), p(452.0, 287.0), p(150.0, 327.0)];
    let narrow_top = [p(210.0, 125.0), p(392.0, 125.0), p(452.0, 327.0), p(150.0, 327.0)];
    let cases = [
        (parallel_sides, 2, p(301.0, 307.0), p(301.0, 347.0), [3, 2], [(111.96, 382.17), (473.9, 311.57)]),
        (parallel_sides, 2, p(301.0, 307.0), p(301.0, 387.0), [3, 2], [(68.2, 442.69), (492.01, 333.93)]),
        (narrow_top, 1, p(422.0, 226.0), p(522.0, 226.0), [1, 2], [(442.4, 79.27), (611.34, 390.69)]),
    ];
    for (quad, edge, from, to, [a, b], want) in cases {
        let got = edge_dragged(quad, edge, from, to);
        let (o, along) = (got.quad[a], got.quad[b] - got.quad[a]);
        for (x, y) in want {
            let off = along.cross(p(x, y) - o) / along.hypot();
            assert!(off.abs() < 0.05, "edge {edge}: ({x}, {y}) is {off} px off the line {:?}", got.quad);
        }
    }
}

/// A plain rectangle scales about the opposite edge's middle as before, and ⇧ still frees the other axis.
#[test]
fn a_plain_edge_drag_and_a_free_one_are_unchanged() {
    let plain = [p(150.0, 125.0), p(452.0, 125.0), p(452.0, 327.0), p(150.0, 327.0)];
    let got = edge_dragged(plain, 1, p(452.0, 226.0), p(502.0, 226.0));
    assert!(within(&got, [(150.0, 108.278), (502.0, 108.278), (502.0, 343.722), (150.0, 343.722)], 2e-3), "{:?}", got.quad);
    let skewed = [p(190.0, 125.0), p(492.0, 125.0), p(452.0, 327.0), p(150.0, 327.0)];
    let start = BoxState::new(skewed, p(321.0, 226.0));
    let free = drag(&start, &start, BoxPart::Edge(2), p(301.0, 327.0), p(301.0, 367.0), Mods { shift: true, ..Mods::NONE }, &BoxRules::default());
    // ⇧ stretches along the side, as the reference does: the skew's offset grows with the height.
    assert!(within(&free, [(190.0, 125.0), (492.0, 125.0), (444.079, 367.0), (142.079, 367.0)], 2e-3), "{:?}", free.quad);
}

/// Only the pointer's move across a dragged side counts (measured on the reference): a right side dragged 50 px out with
/// the pointer also 40 px lower or higher gives the box of the straight drag.
#[test]
fn a_side_drag_in_perspective_ignores_the_pointer_along_the_side() {
    let narrow_top = [p(210.0, 125.0), p(392.0, 125.0), p(452.0, 327.0), p(150.0, 327.0)];
    let straight = edge_dragged(narrow_top, 1, p(422.0, 226.0), p(472.0, 226.0));
    let reference = [(210.0, 97.25), (419.11, 99.99), (526.87, 356.71), (150.0, 354.75)];
    assert!(within(&straight, reference, 3.0), "{:?}", straight.quad);
    for to in [p(472.0, 266.0), p(472.0, 186.0)] {
        let got = edge_dragged(narrow_top, 1, p(422.0, 226.0), to);
        assert!(within(&got, straight.quad.map(|c| (c.x, c.y)), 1e-9), "to {to:?}: {:?}", got.quad);
    }
}

/// Long drags on boxes in strong perspective (measured on the reference): the dragged corners slide along the line
/// through the vanishing point, away from it, to second order in the drag; a side left within 1 px of level is levelled.
#[test]
fn a_long_edge_drag_in_perspective_slides_the_corners_along_the_vanishing_line() {
    let narrow_top = [(210.0, 125.0), (392.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let narrow_bottom = [(150.0, 125.0), (452.0, 125.0), (412.0, 327.0), (190.0, 327.0)];
    let parallel_sides = [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)];
    let cases = [
        (narrow_top, 1, (422.0, 226.0), (522.0, 226.0), [(210.0, 69.51), (442.4, 79.27), (611.34, 390.69), (150.0, 382.49)], 0.2),
        (narrow_top, 1, (422.0, 226.0), (382.0, 226.0), [(210.0, 147.2), (366.79, 149.51), (397.91, 306.0), (150.0, 304.8)], 0.2),
        (narrow_bottom, 1, (432.0, 226.0), (532.0, 226.0), [(150.0, 91.56), (579.75, 89.25), (486.0, 357.74), (190.0, 360.44)], 0.1),
        (parallel_sides, 2, (301.0, 307.0), (301.0, 387.0), [(90.2, 117.08), (511.8, 172.92), (492.01, 333.93), (68.2, 442.69)], 0.5),
        // The top would tilt 0.7 px and the bottom 0.5 px: both stay level, each at its corners' mean.
        (narrow_top, 1, (422.0, 226.0), (447.0, 226.0), [(210.0, 111.49), (406.09, 111.49), (488.34, 341.11), (150.0, 341.11)], 0.02),
    ];
    for (quad, edge, from, to, want, tol) in cases {
        let got = edge_dragged(quad.map(|(x, y)| p(x, y)), edge, p(from.0, from.1), p(to.0, to.1));
        assert!(within(&got, want, tol), "edge {edge} of {quad:?} to {to:?}: {:?}", got.quad);
    }
}

/// ⌥ edge drags (about the reference point, here the box's centre) on boxes whose top and bottom are parallel,
/// measured on the reference: a skewed box keeps its skew offset along a side drag and turns into a rectangle along a
/// top or bottom drag; a box in perspective keeps its narrowing along a side drag and turns into a rectangle along a
/// top or bottom one.
#[test]
fn an_alt_edge_drag_scales_about_the_reference_point_as_the_reference_does() {
    let skewed = [(190.0, 125.0), (492.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let skewed_down = [(150.0, 125.0), (452.0, 165.0), (452.0, 367.0), (150.0, 327.0)];
    let narrow_top = [(210.0, 125.0), (392.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let narrow_bottom = [(150.0, 125.0), (452.0, 125.0), (412.0, 327.0), (190.0, 327.0)];
    let cases = [
        (skewed, 1, (472.0, 226.0), (522.0, 226.0), [(140.0, 91.56), (542.0, 91.56), (502.0, 360.44), (100.0, 360.44)]),
        (skewed, 3, (170.0, 226.0), (130.0, 226.0), [(150.0, 98.25), (532.0, 98.25), (492.0, 353.75), (110.0, 353.75)]),
        (skewed, 2, (301.0, 327.0), (301.0, 367.0), [(138.12, 85.0), (503.88, 85.0), (503.88, 367.0), (138.12, 367.0)]),
        (skewed, 0, (341.0, 125.0), (341.0, 95.0), [(99.21, 95.0), (542.79, 95.0), (542.79, 357.0), (99.21, 357.0)]),
        (skewed_down, 1, (452.0, 266.0), (502.0, 266.0), [(105.14, 86.19), (505.42, 139.21), (496.86, 405.81), (96.58, 352.79)]),
        (skewed_down, 2, (301.0, 347.0), (301.0, 387.0), [(90.2, 77.08), (548.51, 137.78), (511.8, 414.92), (53.49, 354.22)]),
        (narrow_top, 1, (422.0, 226.0), (472.0, 226.0), [(172.4, 83.26), (419.11, 83.26), (526.87, 368.74), (87.6, 368.74)]),
        (narrow_top, 3, (180.0, 226.0), (140.0, 226.0), [(187.98, 91.61), (422.08, 91.61), (501.92, 360.39), (90.82, 360.39)]),
        (narrow_top, 0, (301.0, 125.0), (301.0, 95.0), [(182.97, 95.0), (419.03, 95.0), (419.03, 357.0), (182.97, 357.0)]),
        (narrow_bottom, 1, (432.0, 226.0), (482.0, 226.0), [(92.37, 86.45), (514.37, 86.45), (449.99, 365.55), (147.63, 365.55)]),
        (narrow_bottom, 2, (301.0, 327.0), (301.0, 367.0), [(90.2, 85.0), (511.8, 85.0), (511.8, 367.0), (90.2, 367.0)]),
    ];
    let alt = Mods { alt: true, ..Mods::NONE };
    for (quad, edge, from, to, want) in cases {
        let quad = quad.map(|(x, y)| p(x, y));
        let centre = p(quad.iter().map(|c| c.x).sum::<f64>() / 4.0, quad.iter().map(|c| c.y).sum::<f64>() / 4.0);
        let start = BoxState::new(quad, centre);
        let got = drag(&start, &start, BoxPart::Edge(edge), p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        assert!(within(&got, want, 0.1), "edge {edge} of {quad:?}: {:?}", got.quad);
    }
}

/// An ⌥ corner drag on a box in perspective (measured on the reference): the box scales evenly about the reference point
/// (its centre here), by the pointer's projection on the line from the centre to the corner.
#[test]
fn an_alt_corner_drag_scales_a_box_in_perspective_evenly_about_the_reference_point() {
    let narrow_top = [(210.0, 125.0), (392.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let parallel_sides = [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)];
    let cases = [
        (narrow_top, 2, (452.0, 327.0), (482.0, 357.0), [(189.15, 101.86), (412.85, 101.86), (486.59, 350.14), (115.41, 350.14)]),
        (narrow_top, 0, (210.0, 125.0), (180.0, 95.0), [(181.64, 93.52), (420.36, 93.52), (499.06, 358.48), (102.94, 358.48)]),
        (parallel_sides, 2, (452.0, 287.0), (482.0, 317.0), [(113.79, 100.78), (488.21, 150.37), (488.21, 301.63), (113.79, 351.22)]),
    ];
    let alt = Mods { alt: true, ..Mods::NONE };
    for (quad, corner, from, to, want) in cases {
        let quad = quad.map(|(x, y)| p(x, y));
        let start = BoxState::new(quad, p(301.0, 226.0));
        let got = drag(&start, &start, BoxPart::Corner(corner), p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        assert!(within(&got, want, 0.01), "corner {corner} of {quad:?}: {:?}", got.quad);
    }
}

/// Drags far past the usual range (measured on the reference): a side tripled keeps the dragged corners within 1 px
/// of the reference, and a side shrunk to a sliver, where the second-order rule does not hold, stays near the first
/// step instead of running off.
#[test]
fn far_edge_drags_in_perspective_stay_near_the_reference() {
    let narrow_top = [p(210.0, 125.0), p(392.0, 125.0), p(452.0, 327.0), p(150.0, 327.0)];
    let tripled = edge_dragged(narrow_top, 1, p(422.0, 226.0), p(622.0, 226.0));
    assert!(within(&tripled, [(210.0, 14.01), (480.49, 46.41), (817.96, 474.68), (150.0, 437.99)], 1.0), "{:?}", tripled.quad);
    // Shrunk to under a third, and to a third of a narrower box in the other direction: within 2.5 px.
    let sliver = edge_dragged(narrow_top, 3, p(180.0, 226.0), p(310.0, 226.0));
    let reference = [(310.6, 238.38), (392.0, 197.14), (452.0, 254.86), (310.6, 267.83)];
    let worst = sliver.quad.iter().zip(reference).map(|(a, (x, y))| a.distance(p(x, y))).fold(0.0, f64::max);
    assert!(worst < 2.5, "{worst} px off: {:?}", sliver.quad);
    let narrow_bottom = [p(150.0, 125.0), p(452.0, 125.0), p(412.0, 327.0), p(190.0, 327.0)];
    let shrunk = edge_dragged(narrow_bottom, 1, p(432.0, 226.0), p(282.0, 226.0));
    assert!(within(&shrunk, [(150.0, 175.17), (280.33, 168.02), (283.04, 262.18), (190.0, 276.83)], 0.5), "{:?}", shrunk.quad);
}

/// A Perspective corner drag on a turned box (measured on the reference): the corner moves along the box's top edge,
/// not along the document's axis, and its partner on the top edge the other way.
#[test]
fn a_perspective_corner_on_a_turned_box_moves_along_its_top_edge() {
    let turned = [p(181.5, 89.18), p(473.08, 167.79), p(420.5, 362.82), p(128.92, 284.21)];
    let start = BoxState::new(turned, p(301.0, 226.0));
    let rules = BoxRules::in_mode(TransformMode::Perspective);
    let got = drag(&start, &start, BoxPart::Corner(0), p(182.0, 89.0), p(240.0, 105.0), Mods::NONE, &rules);
    assert!(within(&got, [(239.59, 104.84), (414.99, 152.13), (420.5, 362.82), (128.92, 284.21)], 0.05), "{:?}", got.quad);
}

/// The reference point stays at the corners' mean when a Perspective drag turns a box into a trapezoid (measured on
/// the reference through the ⌥ drag that follows, which scales about it).
#[test]
fn the_reference_point_stays_at_the_corners_mean_through_a_perspective_drag() {
    let start = BoxState::from_rect(Rect::new(150.0, 125.0, 452.0, 327.0));
    let perspective = BoxRules::in_mode(TransformMode::Perspective);
    let trapezoid = drag(&start, &start, BoxPart::Corner(0), p(150.0, 125.0), p(210.0, 125.0), Mods::NONE, &perspective);
    assert!(trapezoid.pivot.distance(p(301.0, 226.0)) < 1e-6, "{:?}", trapezoid.pivot);
    let alt = Mods { alt: true, ..Mods::NONE };
    let got = drag(&trapezoid, &trapezoid, BoxPart::Edge(0), p(301.0, 125.0), p(301.0, 95.0), alt, &BoxRules::default());
    assert!(within(&got, [(182.97, 95.0), (419.03, 95.0), (419.03, 357.0), (182.97, 357.0)], 0.01), "{:?}", got.quad);
}

/// ⌥ drags about a reference point moved off the centre (an ⌥ click at (250, 200) puts it at (249.5, 199.5) on the
/// reference): each scales by how far the dragged part is from the reference point, measured on the reference.
#[test]
fn alt_drags_scale_about_a_moved_reference_point() {
    let rect = [(150.0, 125.0), (452.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let skewed = [(190.0, 125.0), (492.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let narrow_top = [(210.0, 125.0), (392.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let cases = [
        (rect, BoxPart::Edge(1), (452.0, 226.0), (502.0, 226.0), [(125.43, 106.6), (502.0, 106.6), (502.0, 358.48), (125.43, 358.48)]),
        (rect, BoxPart::Corner(2), (452.0, 327.0), (482.0, 357.0), [(132.8, 112.12), (487.01, 112.12), (487.01, 349.04), (132.8, 349.04)]),
        (skewed, BoxPart::Edge(1), (472.0, 226.0), (522.0, 226.0), [(173.64, 108.59), (542.0, 108.59), (502.0, 355.08), (133.64, 355.08)]),
        (narrow_top, BoxPart::Edge(0), (301.0, 125.0), (301.0, 95.0), [(194.09, 95.0), (449.38, 95.0), (449.38, 378.34), (194.09, 378.34)]),
        (narrow_top, BoxPart::Corner(2), (452.0, 327.0), (482.0, 357.0), [(203.17, 112.12), (416.64, 112.12), (487.01, 349.04), (132.8, 349.04)]),
    ];
    let alt = Mods { alt: true, ..Mods::NONE };
    for (quad, part, from, to, want) in cases {
        let quad = quad.map(|(x, y)| p(x, y));
        let start = BoxState::new(quad, p(249.5, 199.5));
        let got = drag(&start, &start, part, p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        // Within 0.1 px: on the skewed box the reference's factor reads 1.2203 where ours reads 1.2195.
        assert!(within(&got, want, 0.1), "{part:?} of {quad:?}: {:?}", got.quad);
        assert_eq!(got.pivot, p(249.5, 199.5), "the reference point stays put");
    }
}

/// An edge dragged past the opposite one (measured on the reference): the box flips along both axes, a rectangle as a
/// skewed box or a box in perspective, the skew's run kept.
#[test]
fn an_edge_dragged_past_the_opposite_one_flips_the_box_both_ways() {
    let rect = [(150.0, 125.0), (452.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let skewed = [(190.0, 125.0), (492.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let narrow_top = [(210.0, 125.0), (392.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let cases = [
        (rect, 2, (301.0, 327.0), (301.0, 67.0), [(344.36, 125.0), (257.64, 125.0), (257.64, 67.0), (344.36, 67.0)]),
        (rect, 1, (452.0, 226.0), (102.0, 226.0), [(150.0, 242.05), (102.0, 242.05), (102.0, 209.95), (150.0, 209.95)]),
        (skewed, 2, (301.0, 327.0), (301.0, 67.0), [(384.36, 125.0), (297.64, 125.0), (257.64, 67.0), (344.36, 67.0)]),
        (skewed, 1, (472.0, 226.0), (122.0, 226.0), [(190.0, 242.05), (142.0, 242.05), (102.0, 209.95), (150.0, 209.95)]),
        (narrow_top, 0, (301.0, 125.0), (301.0, 385.0), [(327.13, 385.0), (274.87, 385.0), (334.87, 327.0), (267.13, 327.0)]),
    ];
    for (quad, edge, from, to, want) in cases {
        let got = edge_dragged(quad.map(|(x, y)| p(x, y)), edge, p(from.0, from.1), p(to.0, to.1));
        assert!(within(&got, want, 0.02), "edge {edge} of {quad:?}: {:?}", got.quad);
    }
}

/// ⌥⇧ drags (free stretch about the reference point, its centre here) on boxes in perspective, measured on the
/// reference: a corner stretches the box along its top edge and across it; an edge and the opposite one each go where
/// a ⇧ drag puts them, the opposite one by the pointer mirrored about the reference point.
#[test]
fn an_alt_shift_drag_stretches_about_the_reference_point() {
    let narrow_top = [(210.0, 125.0), (392.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let parallel_sides = [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)];
    let cases = [
        (narrow_top, BoxPart::Corner(2), (452.0, 327.0), (482.0, 357.0), [(192.0, 95.0), (410.1, 95.0), (482.0, 357.0), (120.1, 357.0)], 0.2),
        (
            parallel_sides,
            BoxPart::Corner(2),
            (452.0, 287.0),
            (482.0, 317.0),
            [(122.19, 69.9), (488.69, 118.44), (482.0, 317.0), (111.12, 398.66)],
            0.05,
        ),
        (narrow_top, BoxPart::Edge(2), (301.0, 327.0), (301.0, 367.0), [(222.0, 85.0), (380.24, 85.0), (464.0, 367.0), (138.24, 367.0)], 0.2),
        // The reference's sides land 0.4 px left of the lines through the pointer and its mirror image.
        (narrow_top, BoxPart::Edge(1), (422.0, 226.0), (472.0, 226.0), [(172.0, 125.0), (429.21, 125.0), (514.0, 327.0), (87.21, 327.0)], 0.5),
    ];
    let alt_shift = Mods { alt: true, shift: true, ..Mods::NONE };
    for (quad, part, from, to, want, tol) in cases {
        let quad = quad.map(|(x, y)| p(x, y));
        let start = BoxState::new(quad, p(301.0, 226.0));
        let got = drag(&start, &start, part, p(from.0, from.1), p(to.0, to.1), alt_shift, &BoxRules::default());
        assert!(within(&got, want, tol), "{part:?} of {quad:?}: {:?}", got.quad);
    }
}

/// ⌥ side drags on a box whose top and bottom converge and whose sides are parallel (measured on the reference): the
/// top and bottom come out parallel, the box's height set by the dragged side; within 1 px of the reference.
#[test]
fn an_alt_side_drag_on_converging_top_and_bottom_levels_them() {
    let parallel_sides = [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)];
    let cases = [
        (1, (452.0, 226.0), (502.0, 226.0), [(97.22, 144.62), (497.67, 197.66), (499.39, 306.67), (111.05, 255.23)]),
        (3, (150.0, 226.0), (110.0, 226.0), [(113.29, 99.03), (494.14, 149.45), (482.07, 352.11), (113.29, 303.25)]),
    ];
    let alt = Mods { alt: true, ..Mods::NONE };
    for (edge, from, to, want) in cases {
        let quad = parallel_sides.map(|(x, y)| p(x, y));
        let start = BoxState::new(quad, p(301.0, 226.0));
        let got = drag(&start, &start, BoxPart::Edge(edge), p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        assert!(within(&got, want, 1.0), "edge {edge}: {:?}", got.quad);
    }
}

/// ⌥ top or bottom drags on a box whose top and bottom converge (measured on the reference): the sides turn across
/// the top edge's frame; within 10 px of the reference at the longest drag (the frame scaling was up to 240 off).
#[test]
fn an_alt_top_drag_on_converging_top_and_bottom_turns_the_sides_across() {
    let parallel_sides = [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)];
    let cases = [
        (2, (301.0, 307.0), (301.0, 347.0), [(79.84, 67.96), (553.21, 149.57), (530.76, 319.1), (39.5, 372.57)], 4.5),
        (2, (301.0, 307.0), (301.0, 387.0), [(10.86, 11.87), (632.0, 131.65), (603.26, 348.66), (-44.23, 427.8)], 10.0),
        (0, (301.0, 145.0), (301.0, 115.0), [(123.76, 82.93), (507.16, 150.79), (485.52, 314.18), (87.54, 356.37)], 0.5),
    ];
    let alt = Mods { alt: true, ..Mods::NONE };
    for (edge, from, to, want, tol) in cases {
        let quad = parallel_sides.map(|(x, y)| p(x, y));
        let start = BoxState::new(quad, p(301.0, 226.0));
        let got = drag(&start, &start, BoxPart::Edge(edge), p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        let worst = got.quad.iter().zip(want).map(|(a, (x, y))| a.distance(p(x, y))).fold(0.0, f64::max);
        assert!(worst < tol, "edge {edge}: {worst} px off: {:?}", got.quad);
    }
}

/// ⌥ side drags on asymmetric boxes in perspective (measured on the reference): the opposite side turns about its
/// middle as well as scaling, by the difference of the two sides' runs.
#[test]
fn an_alt_side_drag_on_an_asymmetric_box_turns_the_opposite_side() {
    let one_side_in = [(190.0, 125.0), (452.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let both_lean = [(200.0, 125.0), (472.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let cases = [
        (one_side_in, 1, (452.0, 226.0), (502.0, 226.0), [(143.55, 89.18), (497.79, 89.18), (506.23, 362.82), (96.45, 362.82)]),
        (one_side_in, 3, (170.0, 226.0), (130.0, 226.0), [(156.25, 96.35), (489.16, 96.35), (494.84, 355.65), (103.61, 355.65)]),
        (one_side_in, 1, (452.0, 226.0), (552.0, 226.0), [(97.09, 53.37), (542.27, 53.37), (561.87, 398.63), (42.91, 398.63)]),
        (both_lean, 1, (462.0, 226.0), (512.0, 226.0), [(152.61, 89.81), (520.74, 89.81), (503.25, 362.19), (97.39, 362.19)]),
        (both_lean, 3, (175.0, 226.0), (135.0, 226.0), [(166.0, 96.85), (509.91, 96.85), (494.09, 355.15), (103.85, 355.15)]),
    ];
    let alt = Mods { alt: true, ..Mods::NONE };
    for (quad, edge, from, to, want) in cases {
        let quad = quad.map(|(x, y)| p(x, y));
        let centre = p(quad.iter().map(|c| c.x).sum::<f64>() / 4.0, quad.iter().map(|c| c.y).sum::<f64>() / 4.0);
        let start = BoxState::new(quad, centre);
        let got = drag(&start, &start, BoxPart::Edge(edge), p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        assert!(within(&got, want, 0.1), "edge {edge} of {quad:?}: {:?}", got.quad);
    }
}

/// ⌥ side drags on boxes whose top and bottom converge and whose sides meet as well (measured on the reference): the
/// same levelling on top of the perspective side rule; a gently distorted box within 0.5 px, one narrowing hard both
/// ways within 8 px.
#[test]
fn an_alt_side_drag_on_a_box_converging_both_ways_levels_top_and_bottom() {
    let one_corner_in = [(180.0, 145.0), (452.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let narrow_and_sloped = [(210.0, 125.0), (392.0, 165.0), (452.0, 327.0), (150.0, 327.0)];
    let cases = [
        (one_corner_in, (452.0, 226.0), (502.0, 226.0), [(128.94, 100.68), (496.04, 73.69), (506.6, 359.95), (102.57, 389.66)], 0.5),
        (narrow_and_sloped, (422.0, 246.0), (472.0, 246.0), [(177.55, 129.85), (408.24, 180.55), (524.38, 364.12), (103.03, 271.51)], 8.0),
    ];
    let alt = Mods { alt: true, ..Mods::NONE };
    for (quad, from, to, want, tol) in cases {
        let quad = quad.map(|(x, y)| p(x, y));
        let centre = p(quad.iter().map(|c| c.x).sum::<f64>() / 4.0, quad.iter().map(|c| c.y).sum::<f64>() / 4.0);
        let start = BoxState::new(quad, centre);
        let got = drag(&start, &start, BoxPart::Edge(1), p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        assert!(within(&got, want, tol), "{quad:?}: {:?}", got.quad);
    }
}

/// Bottom drags that shrink a box in perspective to half or grow it far (measured on the reference): the push's centre
/// follows the bottom-drag law in the start box's frame, for boxes narrowing to the right and to the left.
#[test]
fn a_long_bottom_drag_on_a_box_in_perspective_follows_the_reference() {
    let cases = [
        // Right side 142 of 202, shrunk by 100: was 22.5 px off.
        (
            [(150.0, 125.0), (452.0, 155.0), (452.0, 297.0), (150.0, 327.0)],
            (301.0, 312.0),
            (301.0, 212.0),
            [(224.75, 132.43), (377.25, 147.57), (345.18, 213.5), (216.3, 211.54)],
            1.5,
        ),
        // Right side 52 of 202, grown by 120: was 193 px off.
        (
            [(150.0, 125.0), (452.0, 200.0), (452.0, 252.0), (150.0, 327.0)],
            (301.0, 289.5),
            (301.0, 409.5),
            [(60.3, 102.72), (541.7, 222.28), (476.89, 275.64), (-39.95, 597.17)],
            3.5,
        ),
        // Right side 52 of 202, shrunk by 60: its corners go 3.7 edge lengths, was 241 px off.
        (
            [(150.0, 125.0), (452.0, 200.0), (452.0, 252.0), (150.0, 327.0)],
            (301.0, 289.5),
            (301.0, 229.5),
            [(194.9, 136.1), (407.1, 188.9), (506.1, 227.5), (191.9, 236.5)],
            15.0,
        ),
        // Left side 122 of 202, shrunk by 60: was 15.3 px off.
        (
            [(150.0, 165.0), (452.0, 125.0), (452.0, 327.0), (150.0, 287.0)],
            (301.0, 307.0),
            (301.0, 247.0),
            [(224.26, 155.16), (377.74, 134.84), (384.16, 251.35), (251.84, 246.53)],
            1.0,
        ),
        // Left side 142 of 202, shrunk by 100: was 219 px off.
        (
            [(150.0, 155.0), (452.0, 125.0), (452.0, 327.0), (150.0, 297.0)],
            (301.0, 312.0),
            (301.0, 212.0),
            [(256.34, 144.44), (345.66, 135.56), (363.1, 212.2), (321.25, 212.2)],
            7.5,
        ),
    ];
    for (quad, from, to, want, tol) in cases {
        let got = edge_dragged(quad.map(|(x, y)| p(x, y)), 2, p(from.0, from.1), p(to.0, to.1));
        assert!(within(&got, want, tol), "{quad:?} to {to:?}: {:?}", got.quad);
    }
}

/// A side dragged past the opposite one on a box whose sides converge (measured on the reference): the flipped push
/// levels its factor off, so the far corners land where the reference puts them, for sides narrowing either way.
#[test]
fn a_side_dragged_past_the_opposite_one_in_perspective_follows_the_reference() {
    let narrow_top = [(210.0, 125.0), (392.0, 125.0), (452.0, 327.0), (150.0, 327.0)];
    let narrow_bottom = [(150.0, 125.0), (452.0, 125.0), (376.0, 327.0), (226.0, 327.0)];
    let gentle_bottom = [(150.0, 125.0), (452.0, 125.0), (412.0, 327.0), (190.0, 327.0)];
    let cases = [
        // Was 23.5 px off.
        (narrow_top, (422.0, 226.0), (72.0, 226.0), [(210.0, 319.23), (-22.01, 393.24), (108.84, 160.46), (150.0, 132.77)], 1.0),
        // Was 18 px off.
        (narrow_top, (422.0, 226.0), (92.0, 226.0), [(210.0, 308.13), (28.76, 349.26), (124.79, 162.09), (150.0, 143.87)], 0.5),
        // Was 19.4 px off.
        (narrow_bottom, (414.0, 226.0), (-22.0, 226.0), [(150.0, 270.81), (20.7, 265.7), (-82.57, 169.68), (226.0, 181.19)], 0.5),
        // Was 1.8 px off.
        (gentle_bottom, (432.0, 226.0), (20.0, 226.0), [(150.0, 262.79), (37.15, 266.36), (6.32, 193.8), (190.0, 189.21)], 0.15),
    ];
    for (quad, from, to, want, tol) in cases {
        let got = edge_dragged(quad.map(|(x, y)| p(x, y)), 1, p(from.0, from.1), p(to.0, to.1));
        assert!(within(&got, want, tol), "{quad:?} to {to:?}: {:?}", got.quad);
    }
}

/// ⌥ bottom drags on boxes whose top and bottom converge (measured on the reference): the box grows by less than the
/// pointer's factor, the more the sides differ, for boxes narrowing either way.
#[test]
fn an_alt_bottom_drag_on_converging_edges_eases_its_factor_like_the_reference() {
    let alt = Mods { alt: true, ..Mods::NONE };
    let cases = [
        // Right side 122 of 202, 100 px down: was 12.9 px off.
        (
            [(150.0, 125.0), (452.0, 165.0), (452.0, 287.0), (150.0, 327.0)],
            (301.0, 307.0),
            (301.0, 407.0),
            [(-23.2, -15.82), (670.9, 122.8), (639.11, 362.8), (-85.97, 458.04)],
            0.25,
        ),
        // Right side 82 of 202, 60 px down: was 17.2 px off.
        (
            [(150.0, 125.0), (452.0, 185.0), (452.0, 267.0), (150.0, 327.0)],
            (301.0, 297.0),
            (301.0, 357.0),
            [(40.09, 26.75), (609.59, 185.29), (584.52, 311.46), (-34.83, 403.85)],
            0.5,
        ),
        // Left side 122 of 202, 40 px up: was 2.1 px off.
        (
            [(150.0, 165.0), (452.0, 125.0), (452.0, 327.0), (150.0, 287.0)],
            (301.0, 307.0),
            (301.0, 267.0),
            [(221.36, 184.68), (369.69, 184.68), (382.07, 277.93), (231.08, 258.23)],
            0.3,
        ),
    ];
    for (quad, from, to, want, tol) in cases {
        let quad = quad.map(|(x, y)| p(x, y));
        let mean = p(quad.iter().map(|c| c.x).sum::<f64>() / 4.0, quad.iter().map(|c| c.y).sum::<f64>() / 4.0);
        let start = BoxState::new(quad, mean);
        let got = drag(&start, &start, BoxPart::Edge(2), p(from.0, from.1), p(to.0, to.1), alt, &BoxRules::default());
        assert!(within(&got, want, tol), "{quad:?} to {to:?}: {:?}", got.quad);
    }
}
