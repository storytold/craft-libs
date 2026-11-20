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
