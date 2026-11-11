//! Stories of `IconicColumn`.

use super::CELL;
use crate::buttons::Button;
use crate::contract::ShowContainer as _;
use crate::contract::{Bind, Show as _, Validity};
use crate::docking::{GroupAt, IconicColumn, PanelSpec};
use crate::runtime::env::Message;
use crate::sections::Panel;
use crate::testing::memory::global;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_behavior::dock::{Column, Group};
use egui::Ui;

fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(format!("Panel {panel}"))
}

#[story(IconicColumn<'static, u8>, covers = [Default], drive = Drive::Never)]
fn icon_groups(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    IconicColumn::new("Panels", global("iconic column", "story"), &column, Bind::kept(), &spec)
        .show_flyouts(ui, |group, at, ui, _| {
            let GroupAt::Show(rect) = at else { return None };
            let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
            Panel::new(title).show_at(ui, rect, |_, _| ());
            None
        })
        .into()
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
    let open = data.value("open", None::<u8>);
    let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
    IconicColumn::new("Panels", global("iconic column", "open story"), &column, Bind::bound(open), &spec)
        .show_flyouts(ui, |group, at, ui, _| {
            let GroupAt::Show(rect) = at else { return None };
            let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
            Panel::new(title).show_at(ui, rect, |_, _| ());
            None
        })
        .into()
}

/// A flyout whose content has a focusable control: it takes Tab and its click, and mirrors under right-to-left.
#[story(IconicColumn<'static, u8>, covers = [Open], drive = Drive::Custom(open_first_flyout))]
fn icon_flyout_control(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
    IconicColumn::new("Panels", global("iconic column", "control story"), &column, Bind::kept(), &spec)
        .show_flyouts(ui, |_, at, ui, _| {
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
    IconicColumn::new("Panels", global("iconic column", "value story"), &column, open, &spec)
        .show_flyouts(ui, |_, at, ui, _| {
            let GroupAt::Show(rect) = at else { return None };
            Panel::new("Panel").show_at(ui, rect, |_, _| ());
            None
        })
        .into()
}

#[story(IconicColumn<'static, u8>, covers = [Mixed], drive = Drive::Never)]
fn flyouts_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let open = data.value("open", Mixed::<Option<u8>>::Mixed);
    flyout_of(ui, Bind::bound_mixed(open))
}

#[story(IconicColumn<'static, u8>, covers = [Invalid], drive = Drive::Never)]
fn a_flyout_of_a_panel_not_in_the_column(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let open = data.value("open", Some(9u8));
    let in_column = |open: &Option<u8>| match open {
        Some(panel) if *panel > 1 => Validity::Refuse(Message::new("The column has no such panel")),
        _ => Validity::Ok,
    };
    flyout_of(ui, Bind::bound(open).check(in_column))
}
