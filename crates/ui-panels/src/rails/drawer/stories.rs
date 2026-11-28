//! Stories of `ToolbarDrawer`.

use super::{DrawerGroup, DrawerKind, ToolbarDrawer};
use crate::contract::{Show as _, Tracked};
use crate::display::Text;
use crate::menu::PanelRow;
use crate::rails::tool::stories::{BRUSH, StoryTools, tools};
use crate::rails::tool::{LastUsed, ToolRail, ToolSlot};
use crate::runtime::commands::CommandRef;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

/// The drawer closed beside the tool rail until the rail's Edit Toolbar button opens it, `tune`d by the story. `tune`
/// also learns whether this pass opened the drawer, for a request that is sent once.
fn staged<'a>(
    ui: &mut Ui,
    data: &mut StoryData,
    groups: &'a [DrawerGroup<u8>],
    slots: &'a Tracked<Vec<ToolSlot<u8>>>,
    tune: impl FnOnce(ToolbarDrawer<'a, u8>, bool) -> ToolbarDrawer<'a, u8>,
    below: impl FnMut(&mut Ui),
) -> StoryOutput {
    let last = Tracked::new(LastUsed::default());
    let rail = ToolRail::new("Tools", slots, &BRUSH, &last).edit_toolbar("Edit Toolbar").show_rail(ui, |_, _| {});
    let open = data.value("open", false);
    let Some(opener) = rail.edit_toolbar.as_ref() else { return rail.into() };
    let mut fresh = false;
    if crate::contract::clicked(opener) {
        *open = !*open;
        fresh = *open;
    }
    if !*open {
        return rail.into();
    }
    let drawer = tune(ToolbarDrawer::new(rail.response.rect, groups, slots, rail.dropped), fresh).opened_by(opener).show_with(ui, below);
    if drawer.closed {
        *open = false;
    }
    drawer.into()
}

/// The slots of the toolbar the stories start from.
fn toolbar() -> Tracked<Vec<ToolSlot<u8>>> {
    Tracked::new(vec![ToolSlot::new(0, tools(&[0])), ToolSlot::new(1, tools(&[BRUSH, BRUSH + 1]))])
}

/// The drawer with a header menu, content in the footer above Restore Defaults, and the first group's tools in runs of
/// 2, 1 and 3 rows.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Never)]
fn toolbar_drawer_menu_footer_and_runs(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let groups = [
        DrawerGroup::slotted("Drawing", vec![tools(&[0, 1]), tools(&[2]), tools(&[3, 4, 5])]),
        DrawerGroup::new("Retouching", tools(&[6, 7])),
        DrawerGroup::new("Other", tools(&[8])),
    ];
    let rows =
        vec![PanelRow::Command(CommandRef::from(StoryTools::Move)), PanelRow::Separator, PanelRow::Command(CommandRef::from(StoryTools::Brush))];
    staged(
        ui,
        data,
        &groups,
        &toolbar(),
        |drawer, _| drawer.menu(rows),
        |ui| {
            Text::new("Below").show(ui);
        },
    )
}

/// The drawer with the row of a tool of the last group marked and scrolled into view as it opens.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Never)]
fn toolbar_drawer_marked_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let marked = 4_u8;
    let groups = [DrawerGroup::new("Drawing", tools(&[0, 1, 2])), DrawerGroup::new("Retouching", tools(&[3, 4]))];
    staged(
        ui,
        data,
        &groups,
        &toolbar(),
        |drawer, fresh| {
            let drawer = drawer.mark(&marked);
            if fresh { drawer.reveal(&marked) } else { drawer }
        },
        |_| {},
    )
}

/// The drawer beside the tool rail: the rail's Edit Toolbar button opens and closes it. The eraser and the clone stamp
/// are off the toolbar, so the drawer has tools to drag onto it.
fn beside_rail(ui: &mut Ui, data: &mut StoryData, kind: DrawerKind, open_at_start: bool, grouped_tools_drag: bool) -> StoryOutput {
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
    let drawer = ToolbarDrawer::new(rail.response.rect, &groups, &slots, rail.dropped).opened_by(opener).kind(kind);
    let drawer = if grouped_tools_drag { drawer.grouped_tools_drag() } else { drawer }.show(ui);
    if drawer.closed {
        *open = false;
    }
    drawer.into()
}

/// The drawer as an app shows it: closed beside the tool rail until the rail's Edit Toolbar button opens it.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Never)]
fn toolbar_drawer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    beside_rail(ui, data, DrawerKind::Plain, false, false)
}

/// PdfCraft's drawer (`grouped_tools_drag`): the pencil, grouped behind the brush on the toolbar, still drags to a slot
/// of its own.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Never)]
fn toolbar_drawer_with_grouped_tools_dragging(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    beside_rail(ui, data, DrawerKind::Plain, false, true)
}

/// Types a query into the drawer's search field.
fn type_query(fixture: &mut Fixture) {
    let Some(field) = fixture.by_label("Search Tools") else { return };
    fixture.click(&field).type_text("eraser").idle(3);
}

/// A searchable drawer, open, with a query that keeps the eraser and its group's heading.
#[story(ToolbarDrawer<'static, u8>, covers = [], drive = Drive::Custom(type_query))]
fn searchable_drawer_with_a_query(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    beside_rail(ui, data, DrawerKind::Searchable, true, false)
}
