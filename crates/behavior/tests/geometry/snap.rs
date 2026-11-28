//! Snapping to targets within a tolerance.

use craft_behavior::Axis;
use craft_behavior::geometry::{SnapKind, SnapKinds, SnapTarget, SnapTargets};
use kurbo::{Point, Rect, Vec2};

fn guide(along: Axis, position: f64) -> SnapTarget {
    SnapTarget::new(along, position, SnapKind::Guide)
}

#[test]
fn a_moving_rect_snaps_its_nearest_edge_or_centre() {
    let mut targets = SnapTargets::new();
    targets.add(guide(Axis::Y, 100.0));
    targets.add(guide(Axis::X, 50.0));
    let (offset, _) = targets.snap_rect(Rect::new(76.0, 53.0, 120.0, 90.0), 4.0);
    assert_eq!(offset, Vec2::new(2.0, -3.0));
    let (offset, lines) = targets.snap_rect(Rect::new(10.0, 10.0, 20.0, 20.0), 4.0);
    assert_eq!(offset, Vec2::ZERO);
    assert!(lines.is_empty());
}

/// A rigid shape snaps by its anchors (a transform box moved from inside: its corners and centre), not by the bounds
/// around them: a corner inside the bounds lands on a line, a point that is no anchor (the middle of a slanted side)
/// does not. From closer than the reach only: an anchor exactly the reach away stays.
#[test]
fn a_moved_shape_snaps_by_its_anchors_from_closer_than_the_reach() {
    let mut targets = SnapTargets::new();
    targets.add(guide(Axis::X, 225.0));
    // A slanted box and its centre: the top-right corner (y 221) lies inside the bounds (y 161..423).
    let slanted = [Point::new(190.0, 161.0), Point::new(492.0, 221.0), Point::new(492.0, 423.0), Point::new(190.0, 363.0), Point::new(341.0, 292.0)];
    let (offset, lines) = targets.snap_anchors_of(&slanted, 8.0, SnapKinds::ALL);
    assert_eq!(offset, Vec2::new(0.0, 4.0), "the corner inside the bounds lands on the line");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].position, 225.0);
    // 101 up, the middle of the slanted right side is at 221 and no anchor is near the line.
    let up = slanted.map(|p| p + Vec2::new(0.0, -101.0));
    assert_eq!(targets.snap_anchors_of(&up, 8.0, SnapKinds::ALL), (Vec2::ZERO, Vec::new()));
    // 64 up, only the centre is near the line, 3 below it.
    let centred = slanted.map(|p| p + Vec2::new(0.0, -64.0));
    let (offset, lines) = targets.snap_anchors_of(&centred, 8.0, SnapKinds::ALL);
    assert_eq!((offset, lines.len()), (Vec2::new(0.0, -3.0), 1), "the centre snaps as a corner does");
    let pair = |y: f64| [Point::new(0.0, y), Point::new(10.0, y - 100.0)];
    assert_eq!(targets.snap_anchors_of(&pair(233.0), 8.0, SnapKinds::ALL).0, Vec2::ZERO, "exactly the reach away stays");
    assert_eq!(targets.snap_anchors_of(&pair(232.0), 8.0, SnapKinds::ALL).0, Vec2::new(0.0, -7.0));
    assert_eq!(targets.snap_anchors_of(&pair(232.0), 8.0, SnapKinds::only(SnapKind::Grid)).0, Vec2::ZERO, "the kinds left out do not snap");
    assert_eq!(targets.snap_anchors_of(&[], 8.0, SnapKinds::ALL), (Vec2::ZERO, Vec::new()));
}

#[test]
fn a_moving_rect_skips_the_kinds_left_out() {
    let mut targets = SnapTargets::new();
    targets.add(SnapTarget::new(Axis::Y, 100.0, SnapKind::Selection));
    let rect = Rect::new(76.0, 53.0, 98.0, 90.0);
    let (offset, lines) = targets.snap_rect(rect, 4.0);
    assert_eq!(offset.x, 2.0, "the selection's edge snaps by default");
    assert_eq!(lines.len(), 1);
    let (offset, lines) = targets.snap_rect_of(rect, 4.0, SnapKinds::ALL.without(SnapKind::Selection));
    assert_eq!((offset, lines.len()), (Vec2::ZERO, 0), "a moved selection ignores the selection's edges");
}

#[test]
fn a_spanned_target_catches_only_a_box_that_overlaps_its_span_on_the_other_axis() {
    let mut targets = SnapTargets::new();
    // A layer on page 2: its left edge at x 100 spans y 1200 to 1300.
    targets.add_rect(Rect::new(100.0, 1200.0, 300.0, 1300.0), SnapKind::LayerEdge, SnapKind::LayerCenter);
    targets.add(guide(Axis::Y, 400.0));

    // Over page 1, 2 units right of the edge's line but far from its span: no snap.
    let (offset, lines) = targets.snap_rect(Rect::new(102.0, 50.0, 150.0, 90.0), 4.0);
    assert_eq!((offset, lines.len()), (Vec2::ZERO, 0), "a box over page 1 is not caught by page 2's layer");
    // Beside the layer on page 2: caught.
    let (offset, _) = targets.snap_rect(Rect::new(102.0, 1210.0, 150.0, 1240.0), 4.0);
    assert_eq!(offset.x, -2.0, "a box overlapping the span snaps to the edge");

    // A point the same way.
    let (at, lines) = targets.snap_point(Point::new(102.0, 60.0), 4.0);
    assert_eq!((at, lines.len()), (Point::new(102.0, 60.0), 0));
    let (at, _) = targets.snap_point(Point::new(102.0, 1250.0), 4.0);
    assert_eq!(at.x, 100.0);

    // A full-height guide catches a box anywhere along it.
    let (offset, _) = targets.snap_rect(Rect::new(402.0, 50.0, 450.0, 90.0), 4.0);
    assert_eq!(offset.x, -2.0, "an unspanned target is unchanged");
}

/// With `reach_past_spans` (Photoshop's layer edges), a spanned target catches a box anywhere along its line: boxes
/// side by side align their tops, and the snapped line still reaches from one box to the other.
#[test]
fn reach_past_spans_lets_a_box_beside_a_layer_snap_to_its_edge() {
    let mut targets = SnapTargets::new().reach_past_spans();
    // A layer at x 300..350, y 200..260.
    targets.add_rect(Rect::new(300.0, 200.0, 350.0, 260.0), SnapKind::LayerEdge, SnapKind::LayerCenter);

    // A box left of the layer, its top 3 units above the layer's top: no overlap in x, still caught.
    let (offset, lines) = targets.snap_rect(Rect::new(40.0, 197.0, 90.0, 237.0), 4.0);
    assert_eq!(offset.y, 3.0, "the box's top snaps to the layer's top");
    let line = lines.iter().find(|l| l.along == Axis::X).expect("a horizontal smart guide");
    assert_eq!(line.span, Some((40.0, 350.0)), "the guide runs from the box to the layer");

    // A point the same way.
    let (at, _) = targets.snap_point(Point::new(60.0, 202.0), 4.0);
    assert_eq!(at.y, 200.0);

    // The filtered copy keeps the reach.
    let (offset, _) = targets.filtered(|kind| kind == SnapKind::LayerEdge).snap_rect(Rect::new(40.0, 197.0, 90.0, 237.0), 4.0);
    assert_eq!(offset.y, 3.0, "Snap To with a kind off keeps the reach");
}
