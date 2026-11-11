//! Stories of the transport bar and the media controls.

use super::{MediaControls, TransportBar, TransportBarSettings, TransportButton};
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(TransportBar<'static>, covers = [Default], drive = Drive::Never)]
fn transport_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("settings", TransportBarSettings::default());
    TransportBar::new("Program transport", &TransportButton::PROGRAM).playing(true).looping(true).settings(Bind::bound(settings)).show(ui).into()
}

#[story(MediaControls, covers = [Default], drive = Drive::Never)]
fn media_controls(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    MediaControls::new("Media controls", 12.5, 65.0).trim((5.0, 55.0)).show(ui).into()
}
