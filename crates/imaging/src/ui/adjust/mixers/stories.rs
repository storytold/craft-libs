//! Stories of Black & White, Channel Mixer and Selective Color.

use super::{BlackWhite, BlackWhiteEditor, ChannelMixer, ChannelMixerEditor, SelectiveColor, SelectiveColorEditor};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// Tint on, with its colour well.
#[story(BlackWhiteEditor<'static>, covers = [Default], drive = Drive::Never)]
fn black_white_tinted(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("value", BlackWhite { tint: true, tint_color: Srgba::rgb(0.88, 0.83, 0.7), ..BlackWhite::default() });
    BlackWhiteEditor::new("Black & White", Bind::bound(value)).show(ui).into()
}

/// Monochrome on: the one Gray output, and a Total over 100 % that warns.
#[story(ChannelMixerEditor<'static>, covers = [Default], drive = Drive::Never)]
fn channel_mixer_monochrome(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("value", ChannelMixer { gray: Some([60.0, 60.0, 20.0, 0.0]), ..ChannelMixer::default() });
    ChannelMixerEditor::new("Channel Mixer", Bind::bound(value)).show(ui).into()
}

#[story(SelectiveColorEditor<'static>, covers = [Default], drive = Drive::Never)]
fn selective_color(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = SelectiveColor::default();
    start.ranges[0] = [10.0, -20.0, 30.0, 0.0];
    let value = data.value("value", start);
    SelectiveColorEditor::new("Selective Color", Bind::bound(value)).show(ui).into()
}
