//! Stories of `IconicColumn`.

use super::CELL;
use crate::buttons::Button;
use crate::contract::ShowContainer as _;
use crate::contract::{Bind, BodyCx, Show as _, Validity};
use crate::docking::{ColumnBar, ColumnCards, DockSizes, GroupAt, IconGroupSeparator, IconicColumn, IconicColumnOutput, IconicMode, PanelSpec};
use crate::runtime::commands::{CommandState, CommandStates};
use crate::runtime::env::{Env, Message, with_env};
use crate::sections::Panel;
use crate::tabs::PanelTabLook;
use crate::testing::memory::global;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, stage, story};
use craft_behavior::Mixed;
use craft_behavior::dock::{Column, DockEdge, Group};
use craft_behavior::overlay::Side;
use egui::{Rect, Ui};

/// Panels without icons, so each cell shows the first letter of its panel's name.
fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(["Brushes", "Swatches", "History"].get(usize::from(*panel)).copied().unwrap_or("Panel"))
}

/// The toggle commands of the story panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "iconicStory", story)]
#[non_exhaustive]
pub(crate) enum PanelToggles {
    /// Brushes.
    #[command(label = "Show Brushes")]
    Brushes,
    /// Swatches.
    #[command(label = "Show Swatches")]
    Swatches,
    /// History.
    #[command(label = "Show History")]
    History,
}

/// The story panels, each bound to its toggle command.
fn toggled(panel: &u8) -> PanelSpec {
    let command = [PanelToggles::Brushes, PanelToggles::Swatches, PanelToggles::History][usize::from(*panel) % 3];
    spec(panel).command(command)
}

/// Brushes has something to show while it is closed, so its cell carries the marker; Swatches has something to show
/// but is open, so its cell carries none; History has nothing to show.
fn marked_states(states: &mut CommandStates) {
    states.set(PanelToggles::Brushes, CommandState::new().checked(false).marked(true).hint("3 new brushes"));
    states.set(PanelToggles::Swatches, CommandState::new().checked(true).marked(true).hint("2 new swatches"));
    states.set(PanelToggles::History, CommandState::new().checked(false));
}

/// Shows `column` as a dock shows it: at the end edge of a stage, as wide as the rail, its flyouts opening over the
/// stage.
fn at_stage<'a>(
    ui: &mut Ui,
    column: IconicColumn<'a, u8>,
    view: impl FnMut(&Group<u8>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a,
) -> IconicColumnOutput<u8> {
    at_stage_wider(ui, column, 0.0, view)
}

/// [`at_stage`] with the column `extra` wider than the rail: a card's inset.
fn at_stage_wider<'a>(
    ui: &mut Ui,
    column: IconicColumn<'a, u8>,
    extra: f32,
    view: impl FnMut(&Group<u8>, GroupAt, &mut Ui, &mut BodyCx<'_>) -> Option<f32> + 'a,
) -> IconicColumnOutput<u8> {
    let room = stage(ui);
    let env = Env::of(ui);
    let width = env.tokens.tokens().size.rail.width + extra; // design: allow[style/token-read](a story sizes its stage from the rail token)
    let cell = if env.direction.is_rtl() {
        Rect::from_min_max(room.min, egui::pos2(room.left() + width, room.bottom()))
    } else {
        Rect::from_min_max(egui::pos2(room.right() - width, room.top()), room.max)
    };
    column.edge(DockEdge::End).opens(Side::Start).within(room).show_flyouts_at(ui, cell, view)
}

#[story(IconicColumn<'static, u8>, covers = [Default], drive = Drive::Never)]
fn icon_groups(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "story"), &column, Bind::kept(), &spec);
    at_stage(ui, column, |group, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
        Panel::new(title).titled().show_at(ui, rect, |_, _| ());
        None
    })
    .into()
}

/// A docked column of icons shown as a card (`DockConfig::column_cards`), at the window edge or inward of another
/// column: inset on its top and bottom, and on its outer side by the window edge's inset or by the gutter between two
/// cards, its bar and icons on the rounded panel card over the dock's background.
#[story(IconicColumn<'static, u8>, covers = [], drive = Drive::Never)]
fn icons_as_a_card(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let at = *data.choice("place", 1_usize, &[("At the window edge", 0), ("Inward of another column", 1)]);
    let sizes = DockSizes { column_cards: ColumnCards::All, ..DockSizes::of_scope(ui) };
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    let inset = sizes.card_inset(at, &column, &spec);
    let card = sizes.card_place(at, &column, &spec);
    let column = IconicColumn::new("Panels", global("iconic column", "card story"), &column, Bind::kept(), &spec);
    let column = match card {
        Some(place) => column.card(place),
        None => column,
    };
    at_stage_wider(ui, column, inset, |group, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
        Panel::new(title).titled().show_at(ui, rect, |_, _| ());
        None
    })
    .into()
}

/// The icon groups of an app that sets `IconGroupSeparator::Rule` at its root (VectorCraft's): no grip strips, and an
/// edge rule across the room between two groups.
#[story(IconicColumn<'static, u8>, covers = [], drive = Drive::Never, title = "Icon groups apart by a rule")]
fn icon_groups_apart_by_a_rule(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    with_env(
        ui,
        |overrides| {
            overrides.values.set(IconGroupSeparator::Rule);
        },
        |ui| {
            let column = IconicColumn::new("Panels", global("iconic column", "rule story"), &column, Bind::kept(), &spec);
            at_stage(ui, column, |_, _, _, _| None).into()
        },
    )
}

/// Cells bound to their panels' toggle commands: a marked command whose panel is closed shows the marker.
#[story(IconicColumn<'static, u8>, covers = [Default], drive = Drive::Never, commands = marked_states)]
fn icon_groups_with_marked_panels(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "marked story"), &column, Bind::kept(), &toggled);
    at_stage(ui, column, |group, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
        Panel::new(title).titled().show_at(ui, rect, |_, _| ());
        None
    })
    .into()
}

/// The column of a floating window collapsed to icons: the window's title row is its bar, so the icons start at its
/// top, and its flyouts open towards the room.
#[story(IconicColumn<'static, u8>, covers = [Default], drive = Drive::Never)]
fn icons_in_a_floating_window(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "window story"), &column, Bind::kept(), &spec).bar(ColumnBar::Window);
    at_stage(ui, column, |group, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
        Panel::new(title).titled().show_at(ui, rect, |_, _| ());
        None
    })
    .into()
}

/// The icons as a rail ([`IconicMode::Rail`]) beside their expanded column: the panels in front of their groups
/// show selected, and the bar's chevron collapses the column. A click opens no flyout.
#[story(IconicColumn<'static, u8>, covers = [Default], drive = Drive::Never)]
fn icons_as_a_rail_beside_the_expanded_column(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 1), Group::new(vec![2], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "rail story"), &column, Bind::kept(), &spec).mode(IconicMode::Rail);
    at_stage(ui, column, |_, _, _, _| None).into()
}

/// A click on the first icon opens the flyout of its panel, which the bound value then names.
fn open_first_flyout(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(cell) = fixture.part(id, CELL.at(0)) else { return };
    fixture.click(&cell);
    fixture.idle(3);
}

#[story(IconicColumn<'static, u8>, covers = [Open], drive = Drive::Custom(open_first_flyout))]
fn icon_flyout_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (open, open_at) = data.value_at("open", None::<u8>);
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "open story"), &column, open_at.bind(open), &spec);
    at_stage(ui, column, |group, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
        Panel::new(title).titled().show_at(ui, rect, |_, _| ());
        None
    })
    .into()
}

/// A flyout whose content has a focusable control: it takes Tab and its click, and mirrors under right-to-left.
#[story(IconicColumn<'static, u8>, covers = [Open], drive = Drive::Custom(open_first_flyout))]
fn icon_flyout_control(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "control story"), &column, Bind::kept(), &spec);
    at_stage(ui, column, |_, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        Panel::new("Panel").show_at(ui, rect, |ui, _| {
            Button::new("Apply").show(ui);
        });
        None
    })
    .into()
}

fn flyout_of(ui: &mut Ui, open: Bind<'_, Option<u8>>) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "value story"), &column, open, &spec);
    at_stage(ui, column, |_, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        Panel::new("Panel").show_at(ui, rect, |_, _| ());
        None
    })
    .into()
}

#[story(IconicColumn<'static, u8>, covers = [Mixed], drive = Drive::Never)]
fn flyouts_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (open, open_at) = data.value_at("open", Mixed::<Option<u8>>::Mixed);
    flyout_of(ui, open_at.bind_mixed(open))
}

#[story(IconicColumn<'static, u8>, covers = [Invalid], drive = Drive::Never)]
fn a_flyout_of_a_panel_not_in_the_column(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (open, open_at) = data.value_at("open", Some(9u8));
    let in_column = |open: &Option<u8>| match open {
        Some(panel) if *panel > 1 => Validity::Refuse(Message::new("The column has no such panel")),
        _ => Validity::Ok,
    };
    flyout_of(ui, open_at.bind(open).check(in_column))
}

/// A column of VectorCraft 0.24's dock (`DockConfig::flush_docks`, `PanelTabLook::Block`): framed in `line.border`, its
/// bar on `surface.canvas`, the open panel's icon on `surface.control`, and its flyout square on the panel surface with
/// the same frame and no shadow. The flyout opens from its icon.
#[story(IconicColumn<'static, u8>, covers = [], drive = Drive::Custom(open_first_flyout), title = "A flush dock's column and its flyout")]
fn a_flush_docks_column_and_its_flyout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (open, open_at) = data.value_at("open", None::<u8>);
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    let column = IconicColumn::new("Panels", global("iconic column", "flush story"), &column, open_at.bind(open), &spec)
        .framed(true)
        .tab_look(PanelTabLook::Block);
    at_stage(ui, column, |group, at, ui, _| {
        let GroupAt::Show(rect) = at else { return None };
        let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
        Panel::new(title).titled().show_at(ui, rect, |_, _| ());
        None
    })
    .into()
}
