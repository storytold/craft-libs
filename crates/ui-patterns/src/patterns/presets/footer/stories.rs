//! Stories of `PresetFooter`.

use super::{PresetFooter, PresetFooterOptions};
use crate::contract::Bind;
use crate::sections::FooterAction;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use egui::Ui;

#[story(PresetFooter<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_footer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let slider = *data.value("slider", true);
    let new_group = *data.value("new_group", true);
    let count = *data.value("count", true);
    let footer = PresetFooter::<u32>::new(Bind::bound(data.value("zoom", 1.0f64))).options(
        PresetFooterOptions::new()
            .slider(slider)
            .new_group(new_group)
            .can_delete(false)
            .action(FooterAction::new(IconId::new("settings"), "Preset settings")),
    );
    let footer = if count { footer.count(12) } else { footer };
    footer.show(ui).into()
}
