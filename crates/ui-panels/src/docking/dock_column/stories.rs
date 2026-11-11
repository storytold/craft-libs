//! Stories of `DockColumn`.

use crate::contract::ShowContainer as _;
use crate::docking::{DockColumn, PanelSpec};
use crate::sections::Panel;
use crate::testing::memory::global;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::dock::{Column, Group};
use egui::Ui;

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
