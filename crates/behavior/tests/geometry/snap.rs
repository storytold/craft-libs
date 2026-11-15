//! Snapping to targets within a tolerance.

use craft_behavior::Axis;
use craft_behavior::geometry::{SnapKind, SnapKinds, SnapTarget, SnapTargets};
use kurbo::{Rect, Vec2};

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
