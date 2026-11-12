//! Stories of `ToolbarDrawer`.

use super::{DrawerGroup, ToolbarDrawer};
use crate::rails::tool::stories::{BRUSH, tools};
use crate::rails::tool::{LastUsed, ToolRail, ToolSlot};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The drawer as an app shows it: closed beside the tool rail until the rail's Edit Toolbar button opens it. The eraser
/// and the clone stamp are off the toolbar, so the drawer has tools to drag onto it.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Never)]
fn toolbar_drawer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let groups = [DrawerGroup::new("Drawing", tools(&[0, 1, 2])), DrawerGroup::new("Retouching", tools(&[3, 4]))];
    let slots = [ToolSlot::new(0, tools(&[0])), ToolSlot::new(1, tools(&[BRUSH, BRUSH + 1]))];
    let last = LastUsed::default();
    let rail = ToolRail::new("Tools", &slots, &BRUSH, &last).edit_toolbar("Edit Toolbar").show_rail(ui, |_, _| {});
    let open = data.value("open", false);
    let Some(opener) = rail.edit_toolbar.as_ref() else { return rail.into() };
    if crate::contract::clicked(opener) {
        *open = !*open;
    }
    if !*open {
        return rail.into();
    }
    let drawer = ToolbarDrawer::new(rail.response.rect, &groups, &slots, rail.dropped).opened_by(opener).show(ui);
    if drawer.closed {
        *open = false;
    }
    drawer.into()
}
