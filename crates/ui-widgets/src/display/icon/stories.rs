//! Stories of `Icon`.

use super::Icon;
use crate::contract::{Props as _, Show as _};
use crate::primitives::IconSpec;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use egui::Ui;

#[story(Icon<'static>, covers = [Default], drive = Drive::Never)]
fn icon(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Icon::new(IconSpec::svg(IconId::new("settings"))).label("Settings").show(ui).into()
}
