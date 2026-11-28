//! Stories of the Tabs dialog.

use super::TabStopsDialog;
use crate::ui::length::DocUnit;
use crate::ui::ruler::{TabAlign, TabLeader, TabStop};
use craft_ui::dialog::{DialogStack, StackDialog as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(TabStopsDialog, covers = [Default], drive = Drive::Never)]
fn with_stops(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let stops = vec![
        TabStop { position: 72.0, align: TabAlign::Left, leader: TabLeader::None },
        TabStop { position: 216.0, align: TabAlign::Right, leader: TabLeader::Dot },
        TabStop { position: 360.0, align: TabAlign::Decimal, leader: TabLeader::Underscore },
    ];
    let mut dialog = TabStopsDialog::new(stops, 36.0).unit(DocUnit::new(UnitKind::Inches, 96.0));
    dialog.select(Some(1));
    let _ = dialog.show(ui, &DialogStack::default());
    StoryOutput::default()
}
