//! Stories of `DockColumn`.

use crate::contract::{ShowContainer as _, Tracked};
use crate::docking::{CardInset, ColumnBar, ColumnCards, DockColumn, DockGrid, DockSizes, PanelSpec};
use crate::rails::tool::stories::{BRUSH, tools};
use crate::rails::{LastUsed, RailDensity, ToolRail, ToolSlot};
use crate::runtime::env::Env;
use crate::sections::Panel;
use crate::tabs::PanelTabLook;
use crate::testing::memory::global;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Flow;
use craft_behavior::dock::{Column, Group};
use egui::{Ui, UiBuilder};

/// The height of the card stories' columns.
const CARD_HEIGHT: f32 = 160.0;

/// The width of the panel card story's column, before its card's inset.
const CARD_WIDTH: f32 = 240.0;

fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(format!("Panel {panel}"))
}

#[story(DockColumn<'static, u8>, covers = [Default], drive = Drive::Never)]
fn two_groups(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    DockColumn::new("Panels", global("dock column", "story"), &column, &spec)
        .show_groups(ui, |group, rect, ui, _| {
            let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
            Panel::new(title).show_at(ui, rect, |_, _| ());
        })
        .into()
}

/// The column of a floating window: the window's title row is its bar, so it shows its groups and the dividers
/// between them only.
#[story(DockColumn<'static, u8>, covers = [Default], drive = Drive::Never)]
fn in_a_floating_window(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    DockColumn::new("Panels", global("dock column", "window story"), &column, &spec)
        .bar(ColumnBar::Window)
        .show_groups(ui, |group, rect, ui, _| {
            let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
            Panel::new(title).show_at(ui, rect, |_, _| ());
        })
        .into()
}

/// A docked column of a tool rail at the window edge shown as a card (`DockConfig::column_cards`), at each rail density
/// and one or two steps: inset on its outer, top and bottom sides, where the dock's background shows, with its bar and
/// its rail on the rounded panel card. The column is wider than its steps by the inset. Beside block tabs
/// (`PanelTabLook::Block`, VectorCraft's) its bar is a square `surface.canvas` strip across the card's top.
#[story(DockColumn<'static, u8>, covers = [], drive = Drive::Never)]
fn a_tool_rail_card(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let density = *data.choice("density", RailDensity::Dense, &[("Regular", RailDensity::Regular), ("Dense", RailDensity::Dense)]);
    let steps = *data.ranged("steps", 1_u16, 1.0..=2.0);
    let look = *data.choice("tabs", PanelTabLook::Plain, &[("Plain", PanelTabLook::Plain), ("Block", PanelTabLook::Block)]);
    let slots = Tracked::new((0..4_u8).map(|slot| ToolSlot::new(u32::from(slot), tools(&[slot]))).collect::<Vec<_>>());
    let last = Tracked::new(LastUsed::default());
    let cells = slots.len();
    let spec = move |_: &u8| PanelSpec::new("Tools").grid(DockGrid::rail(cells).density(density));
    let column = Column::new(vec![Group::new(vec![0], 0)]).with_steps(steps);
    let sizes = DockSizes { column_cards: ColumnCards::All, ..DockSizes::of_scope(ui) };
    let width = sizes.natural(std::slice::from_ref(&column), &spec).first().map_or(0.0, |(width, _)| *width);
    let room = Flow::new(ui.available_rect_before_wrap(), Env::of(ui).direction.is_rtl()).span(0.0, width);
    let rect = room.with_max_y(room.top() + CARD_HEIGHT);
    // The column takes the cell a dock gives it, as high as the dock, whatever its rail's height.
    let shown = DockColumn::new("Panels", global("dock column", "rail card story"), &column, &spec)
        .card(CardInset::Edge)
        .tab_look(look)
        .show_groups_at(ui, rect, |_, rect, ui, _| {
            let rail = ToolRail::new("Tools", &slots, &BRUSH, &last).density(density);
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| rail.show_rail(ui, |_, _| {}));
        });
    ui.advance_cursor_after_rect(rect);
    shown.into()
}

/// A docked panel column of two groups shown as a card (`DockConfig::column_cards`), at the window edge or inward of
/// another column: inset on its top and bottom, and on its outer side by the window edge's inset or by the gutter
/// between two cards, its bar and groups on the rounded panel card with no frame.
#[story(DockColumn<'static, u8>, covers = [], drive = Drive::Never)]
fn a_panel_column_card(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = *data.choice("place", 0_usize, &[("At the window edge", 0), ("Inward of another column", 1)]);
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    let sizes = DockSizes { column_cards: ColumnCards::All, ..DockSizes::of_scope(ui) };
    let inset = sizes.card_inset(at, &column, &spec);
    let room = Flow::new(ui.available_rect_before_wrap(), Env::of(ui).direction.is_rtl()).span(0.0, CARD_WIDTH + inset);
    let rect = room.with_max_y(room.top() + 2.0 * CARD_HEIGHT);
    let shown = DockColumn::new("Panels", global("dock column", "panel card story"), &column, &spec);
    let shown = match sizes.card_place(at, &column, &spec) {
        Some(place) => shown.card(place),
        None => shown,
    };
    let shown = shown.show_groups_at(ui, rect, |group, rect, ui, _| {
        let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
        Panel::new(title).show_at(ui, rect, |_, _| ());
    });
    ui.advance_cursor_after_rect(rect);
    shown.into()
}
