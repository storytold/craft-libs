//! Dock columns: room sharing between groups.

use craft_ui::docking::{DockHeights, GroupSize, group_heights};
use craft_ui::testing::trial;

const STRIP: f32 = 20.0;
const MIN: f32 = 60.0;
const ROOM: f32 = 400.0;

fn open(min: f32) -> GroupSize {
    GroupSize::new(min)
}

fn sum(heights: &[f32]) -> f32 {
    heights.iter().sum()
}

#[trial]
fn default_heights_give_way_to_the_group_that_fills_before_it_gets_less() {
    let mut roomy = open(MIN);
    roomy.content = Some(DockHeights::new(250.0).compact(100.0));
    let mut filling = open(MIN);
    filling.content = Some(DockHeights::new(0.0).fill(200.0));
    let heights = group_heights(&[roomy, filling], STRIP, ROOM);
    assert_eq!(heights[1], 200.0, "the filling group gets its fill height");
    assert_eq!(heights[0], ROOM - 200.0, "the other group gave way from its default of 250");
    assert!(sum(&heights) <= ROOM, "the groups fit the room");
}
