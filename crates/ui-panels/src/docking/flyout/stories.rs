//! Stories of `Flyout`.

use super::Flyout;
use crate::contract::Bind;
use crate::contract::ShowContainer as _;
use crate::docking::{GroupAt, IconicColumn, PanelSpec};
use crate::sections::Panel;
use crate::testing::memory::global;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::dock::{Column, Group};
use egui::Ui;

fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(format!("Panel {panel}"))
}

/// The flyout of the first icon, open from the first pass.
#[story(Flyout, covers = [Default], drive = Drive::Never)]
fn open_beside_its_column(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
    IconicColumn::new("Panels", global("flyout", "story"), &column, Bind::given(Some(0u8)), &spec)
        .show_flyouts(ui, |group, at, ui, _| {
            let GroupAt::Show(rect) = at else { return None };
            let title = group.active_panel().map_or_else(String::new, |panel| spec(panel).label.as_str().to_owned());
            Panel::new(title).show_at(ui, rect, |_, _| ());
            None
        })
        .into()
}
