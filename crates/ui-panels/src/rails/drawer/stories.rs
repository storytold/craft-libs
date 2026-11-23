//! Stories of `ToolbarDrawer`.

use super::{DrawerGroup, DrawerKind, ToolbarDrawer};
use crate::contract::Tracked;
use crate::rails::tool::stories::{BRUSH, tools};
use crate::rails::tool::{LastUsed, ToolRail, ToolSlot};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

/// The drawer beside the tool rail: the rail's Edit Toolbar button opens and closes it. The eraser and the clone stamp
/// are off the toolbar, so the drawer has tools to drag onto it.
fn beside_rail(ui: &mut Ui, data: &mut StoryData, kind: DrawerKind, open_at_start: bool) -> StoryOutput {
    let groups = [DrawerGroup::new("Drawing", tools(&[0, 1, 2])), DrawerGroup::new("Retouching", tools(&[3, 4]))];
    let slots = Tracked::new(vec![ToolSlot::new(0, tools(&[0])), ToolSlot::new(1, tools(&[BRUSH, BRUSH + 1]))]);
    let last = Tracked::new(LastUsed::default());
    let rail = ToolRail::new("Tools", &slots, &BRUSH, &last).edit_toolbar("Edit Toolbar").show_rail(ui, |_, _| {});
    let open = data.value("open", open_at_start);
    let Some(opener) = rail.edit_toolbar.as_ref() else { return rail.into() };
    if crate::contract::clicked(opener) {
        *open = !*open;
    }
    if !*open {
        return rail.into();
    }
    let drawer = ToolbarDrawer::new(rail.response.rect, &groups, &slots, rail.dropped).opened_by(opener).kind(kind).show(ui);
    if drawer.closed {
        *open = false;
    }
    drawer.into()
}

/// The drawer as an app shows it: closed beside the tool rail until the rail's Edit Toolbar button opens it.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Never)]
fn toolbar_drawer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    beside_rail(ui, data, DrawerKind::Plain, false)
}

/// Types a query into the drawer's search field.
fn type_query(fixture: &mut Fixture) {
    let Some(field) = fixture.by_label("Search Tools") else { return };
    fixture.click(&field).type_text("eraser").idle(3);
}

/// A searchable drawer, open, with a query that keeps the eraser and its group's heading.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Custom(type_query))]
fn searchable_drawer_with_a_query(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    beside_rail(ui, data, DrawerKind::Searchable, true)
}
