//! Snapping to targets within a tolerance.

use craft_behavior::Axis;
use craft_behavior::geometry::{SnapKind, SnapTarget, SnapTargets};
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
