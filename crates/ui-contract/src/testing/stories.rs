//! Stories of the kit's stand-in `Block` and of the `Draw::face_run` card.

use super::{Drive, StoryData, StoryOutput, story};
use crate::contract::Show as _;
use crate::testing::face_runs::FaceRuns;
use crate::testing::layout::Block;
use egui::{Ui, Vec2};

/// The size of a trial block, in points.
const CELL: Vec2 = Vec2::new(40.0, 20.0);

#[story(Block, covers = [Default])]
fn block(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Block::new(CELL).show(ui).into()
}

// Text in a document's own face (`Draw::face_run`, an input method's composition): at its size on its baseline, kana
// the face lacks drawn by the interface fonts, and a run asked larger than the font atlas holds drawn at the cap.
#[story(FaceRuns, covers = [Default], drive = Drive::Never, title = "Face runs")]
fn face_runs(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    FaceRuns::new().show(ui).into()
}
