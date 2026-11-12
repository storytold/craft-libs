//! Stories of the Levels panel in one card: the composite with the point pickers, after Auto, a picker armed, a
//! grayscale document, and an isolated mask channel.

use super::LevelsPanel;
use crate::ui::adjust::{AdjustContext, Levels, ToneChannel, ToneChannels, TonePicker, ToneSpace};
use crate::ui::curve::stories::story_bins;
use crate::ui::histogram::ChannelColor;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The host's values of the panel story.
#[derive(Clone, Debug, PartialEq)]
struct Rig {
    levels: Vec<Levels>,
    armed: Option<TonePicker>,
}

fn counts(shift: f32) -> [u32; 256] {
    let mut counts = [0u32; 256];
    for (slot, bin) in counts.iter_mut().zip(story_bins(shift)) {
        *slot = bin;
    }
    counts
}

/// The panel over `channels` with the point pickers, in `context`.
fn panel(ui: &mut Ui, data: &mut StoryData, channels: ToneChannels, context: AdjustContext, histograms: &[[u32; 256]], start: Rig) -> StoryOutput {
    let rig = data.value("rig", start);
    LevelsPanel::new("Levels", Bind::bound(&mut rig.levels), channels)
        .histograms(histograms)
        .pickers(Bind::bound(&mut rig.armed))
        .context(context)
        .show(ui)
        .into()
}

fn rgb_panel(ui: &mut Ui, data: &mut StoryData, levels: Vec<Levels>, armed: Option<TonePicker>) -> StoryOutput {
    let histograms = [counts(0.0), counts(0.0), counts(12.0), counts(-12.0)];
    panel(ui, data, ToneChannels::of(ToneSpace::Rgb, false), AdjustContext::dialog(true), &histograms, Rig { levels, armed })
}

#[story(LevelsPanel<'static>, covers = [Default], drive = Drive::Never, group = "Channels", title = "RGB with a histogram")]
fn levels_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    rgb_panel(ui, data, vec![Levels::identity(0.0..=255.0); 4], None)
}

#[story(LevelsPanel<'static>, covers = [], drive = Drive::Never, group = "Channels", title = "After Auto")]
fn levels_panel_after_auto(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut levels = vec![Levels::identity(0.0..=255.0); 4];
    levels[0] = levels[0].auto(&counts(0.0));
    rgb_panel(ui, data, levels, None)
}

#[story(LevelsPanel<'static>, covers = [], drive = Drive::Never, group = "Channels", title = "Black point picker armed")]
fn levels_panel_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    rgb_panel(ui, data, vec![Levels::identity(0.0..=255.0); 4], Some(TonePicker::Black))
}

#[story(LevelsPanel<'static>, covers = [], drive = Drive::Never, group = "Channels", title = "Grayscale document")]
fn levels_panel_gray(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let start = Rig { levels: vec![Levels::identity(0.0..=255.0)], armed: None };
    panel(ui, data, ToneChannels::of(ToneSpace::Rgb, true), AdjustContext::dialog(true).gray(true), &[counts(0.0)], start)
}

#[story(LevelsPanel<'static>, covers = [], drive = Drive::Never, group = "Channels", title = "One isolated mask channel")]
fn levels_panel_isolated_mask(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let levels = data.value("mask", vec![Levels::identity(0.0..=255.0)]);
    let channels = ToneChannels::isolated(ToneChannel::Named { name: "Layer mask".to_owned(), color: ChannelColor::Neutral });
    LevelsPanel::new("Levels", Bind::bound(levels), channels).histograms(&[counts(0.0)]).show(ui).into()
}
