//! Tests of the Curvature Pen's handles.

use craft_ui::testing::trial;
use kurbo::Point;

use super::curvature::{carried_handles, curvature_handles};

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

// An inner anchor's handles are collinear, parallel to the neighbours' chord, and a third of the
// distance to each neighbour long (uneven spacing).
#[trial]
fn curvature_handles_are_collinear_and_one_third_of_the_neighbour_distance() {
    let a = [p(0.0, 0.0), p(30.0, 40.0), p(120.0, 40.0), p(150.0, 0.0)];
    let h = curvature_handles(&a, &[], false);
    assert_eq!(h.len(), 4);
    let (i, o) = h[1];
    let chord = a[2] - a[0];
    let cross = |u: kurbo::Vec2, v: kurbo::Vec2| u.x * v.y - u.y * v.x;
    assert!(cross(o - a[1], chord).abs() < 1e-9);
    assert!(cross(i - a[1], chord).abs() < 1e-9);
    assert!((a[1].distance(i) - a[1].distance(a[0]) / 3.0).abs() < 1e-9);
    assert!((a[1].distance(o) - a[1].distance(a[2]) / 3.0).abs() < 1e-9);
    assert!((a[1] - i).dot(o - a[1]) > 0.0);
}

// A path carried on keeps its kept knots: extended and open, the run restarts at the last kept knot, whose out handle
// is the new run's; closed, knot 0's in handle bends into the closing segment and goes back in line with its kept out
// handle, else with the next kept knot's in handle (#10726, #10744).
#[trial]
fn a_carried_run_keeps_the_kept_knots_and_bends_knot_0_in_line_when_closed() {
    let anchors = [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0), p(0.0, 100.0)];
    let kept = [(p(-10.0, 5.0), p(30.0, -5.0)), (p(70.0, -3.0), p(130.0, 3.0))];

    let open = carried_handles(&anchors, &[], false, &kept);
    assert_eq!(open.len(), 4);
    assert_eq!(open[0], kept[0], "knots before the last kept one keep both handles");
    assert_eq!(open[1].0, kept[1].0, "the last kept knot keeps its in handle");
    // The new run is open from knot 1: its first anchor's out handle and the knots after it are the run's.
    let run = curvature_handles(&anchors[1..], &[], false);
    assert_eq!(open[1].1, run[0].1, "the last kept knot's out handle starts the new run");
    assert_ne!(open[1].1, kept[1].1);
    assert_eq!(&open[2..], &run[1..]);

    // Closed: not extended, so the kept out handles stay; knot 0's in handle is recomputed, then put in line with its
    // kept out handle at the recomputed handle's length.
    let all = [(p(-10.0, 5.0), p(30.0, -5.0)), (p(70.0, -3.0), p(130.0, 3.0)), (p(100.0, 70.0), p(100.0, 130.0)), (p(30.0, 100.0), p(-30.0, 100.0))];
    let closed = carried_handles(&anchors, &[], true, &all);
    assert_eq!(closed[0].1, all[0].1);
    assert_eq!(closed[1], all[1]);
    let bent = curvature_handles(&anchors, &[], true)[0].0;
    let (anchor, out) = (anchors[0], all[0].1);
    let want = anchor + (anchor - out) / anchor.distance(out) * anchor.distance(bent);
    assert!(closed[0].0.distance(want) < 1e-9, "{:?} vs {want:?}", closed[0].0);

    // An out handle on its anchor gives no direction: the next kept knot's in handle does.
    let mut flat = all;
    flat[0].1 = anchors[0];
    let closed = carried_handles(&anchors, &[], true, &flat);
    let toward = flat[1].0;
    let want = anchor + (anchor - toward) / anchor.distance(toward) * anchor.distance(bent);
    assert!(closed[0].0.distance(want) < 1e-9, "{:?} vs {want:?}", closed[0].0);
}
