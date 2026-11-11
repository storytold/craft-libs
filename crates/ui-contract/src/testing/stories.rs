//! Stories of the kit's stand-in `Block`.

use super::{StoryData, StoryOutput, story};
use crate::contract::Show as _;
use crate::testing::layout::Block;
use egui::{Ui, Vec2};

/// The size of a trial block, in points.
const CELL: Vec2 = Vec2::new(40.0, 20.0);

#[story(Block, covers = [Default])]
fn block(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Block::new(CELL).show(ui).into()
}
