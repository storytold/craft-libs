//! Stories of `ToolbarDrawer`.

use super::{DrawerGroup, ToolbarDrawer};
use crate::rails::tool::ToolSlot;
use crate::rails::tool::stories::tools;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Never)]
fn toolbar_drawer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let groups = [DrawerGroup::new("Drawing", tools(&[0, 1, 2])), DrawerGroup::new("Retouching", tools(&[3, 4]))];
    let toolbar = [ToolSlot::new(0, tools(&[0])), ToolSlot::new(1, tools(&[1, 2]))];
    ToolbarDrawer::new(ui.max_rect(), &groups, &toolbar, None).show(ui).into()
}
