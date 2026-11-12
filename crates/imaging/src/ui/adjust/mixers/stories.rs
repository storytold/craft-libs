//! Stories of Black & White, Channel Mixer and Selective Color: each editor's cases in one card.

use super::{BlackWhite, BlackWhiteEditor, ChannelMixer, ChannelMixerEditor, SelectiveColor, SelectiveColorEditor, SelectiveMethod};
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn black_white(ui: &mut Ui, data: &mut StoryData, start: BlackWhite) -> StoryOutput {
    let value = data.value("value", start);
    BlackWhiteEditor::new("Black & White", Bind::bound(value)).show(ui).into()
}

#[story(BlackWhiteEditor<'static>, covers = [Default], drive = Drive::Never, group = "Tint", title = "Default")]
fn black_white_default(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    black_white(ui, data, BlackWhite::default())
}

/// Tint on, with its colour well.
#[story(BlackWhiteEditor<'static>, covers = [], drive = Drive::Never, group = "Tint", title = "Tinted")]
fn black_white_tinted(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    black_white(ui, data, BlackWhite { weights: [60.0, 80.0, 30.0, 70.0, 10.0, 40.0], tint: true, ..BlackWhite::default() })
}

fn channel_mixer(ui: &mut Ui, data: &mut StoryData, start: ChannelMixer) -> StoryOutput {
    let value = data.value("value", start);
    ChannelMixerEditor::new("Channel Mixer", Bind::bound(value)).show(ui).into()
}

#[story(ChannelMixerEditor<'static>, covers = [Default], drive = Drive::Never, group = "Outputs", title = "Color")]
fn channel_mixer_color(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    channel_mixer(ui, data, ChannelMixer { red: [80.0, 20.0, 0.0, 0.0], ..ChannelMixer::default() })
}

/// Monochrome on: the one Gray output.
#[story(ChannelMixerEditor<'static>, covers = [], drive = Drive::Never, group = "Outputs", title = "Monochrome")]
fn channel_mixer_monochrome(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    channel_mixer(ui, data, ChannelMixer { gray: Some(ChannelMixer::MONOCHROME_START), ..ChannelMixer::default() })
}

/// A Total over 100 % warns.
#[story(ChannelMixerEditor<'static>, covers = [], drive = Drive::Never, group = "Outputs", title = "Total over 100 %")]
fn channel_mixer_over(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    channel_mixer(ui, data, ChannelMixer { red: [70.0, 50.0, 0.0, 0.0], ..ChannelMixer::default() })
}

fn selective_color(ui: &mut Ui, data: &mut StoryData, start: SelectiveColor) -> StoryOutput {
    let value = data.value("value", start);
    SelectiveColorEditor::new("Selective Color", Bind::bound(value)).show(ui).into()
}

#[story(SelectiveColorEditor<'static>, covers = [Default], drive = Drive::Never, group = "Methods", title = "Relative")]
fn selective_color_relative(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = SelectiveColor::default();
    start.ranges[0] = [30.0, -10.0, 20.0, 0.0];
    selective_color(ui, data, start)
}

#[story(SelectiveColorEditor<'static>, covers = [], drive = Drive::Never, group = "Methods", title = "Absolute")]
fn selective_color_absolute(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut start = SelectiveColor { method: SelectiveMethod::Absolute, ..SelectiveColor::default() };
    start.ranges[0] = [-20.0, 25.0, 0.0, 10.0];
    selective_color(ui, data, start)
}
