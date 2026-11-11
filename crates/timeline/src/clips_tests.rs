//! Clip edges, trims and snapping.

use crate::clips::{ClipEdge, EdgeZones, edge_at};
use crate::model::ClipId;
use crate::time::{TICKS_PER_SECOND, Tick, TimeRange, TimeView};
use craft_ui::testing::trial;

const ZONES: EdgeZones = EdgeZones { edge: 7.0, roll: 2.5 };

fn seconds(s: i64) -> Tick {
    Tick(s * TICKS_PER_SECOND)
}

fn range(from: i64, to: i64) -> TimeRange {
    TimeRange::from_bounds(seconds(from), seconds(to))
}

/// 10 points a second from zero.
fn view() -> TimeView {
    TimeView::new(Tick::ZERO, 10.0)
}

// edge_zone is the edge size, at most a third of the width, None for zero width.

// edge_at prefers the edge whose clip the pointer is inside.
#[trial]
fn the_clip_under_the_pointer_wins_a_shared_cut() {
    let clips = [(ClipId(2), range(5, 10)), (ClipId(1), range(0, 5))];
    // the cut is at x = 50
    let left_side = edge_at(&clips, &view(), 0.0, 48.0, &ZONES).expect("edge");
    assert_eq!((left_side.clip, left_side.edge, left_side.inside), (ClipId(1), ClipEdge::Out, true));
    let right_side = edge_at(&clips, &view(), 0.0, 52.0, &ZONES).expect("edge");
    assert_eq!((right_side.clip, right_side.edge, right_side.inside), (ClipId(2), ClipEdge::In, true));
    let on_the_cut = edge_at(&clips, &view(), 0.0, 50.0, &ZONES).expect("edge");
    assert_eq!((on_the_cut.clip, on_the_cut.edge), (ClipId(2), ClipEdge::In));
    assert_eq!(edge_at(&clips, &view(), 0.0, 25.0, &ZONES), None);
}

// Cmd+Shift roll, Cmd ripple, no-modifier roll on the cut and ripple elsewhere.

// snap_time snaps within the tolerance and reports the target.
