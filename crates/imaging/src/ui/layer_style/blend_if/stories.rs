//! Stories of `BlendIfBar`: halves together (one triangle per pair) and halves apart (split handles).

use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::BlendIfBar;
use crate::layer_style::BlendIfRange;

#[story(BlendIfBar<'static>, covers = [], drive = Drive::Never, group = "Handles", title = "Unsplit pairs")]
fn blend_if_bar_unsplit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let range = data.value("range", BlendIfRange([20, 20, 240, 240]));
    BlendIfBar::new("Underlying Layer", Bind::bound(range)).show(ui).into()
}

#[story(BlendIfBar<'static>, covers = [Default], drive = Drive::Never, group = "Handles", title = "Split pairs (Alt-drag splits a pair)")]
fn blend_if_bar_split(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let range = data.value("range", BlendIfRange([10, 40, 200, 230]));
    BlendIfBar::new("This Layer", Bind::bound(range)).show(ui).into()
}
