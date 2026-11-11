//! Stories of the Levels editor: RGB with a histogram, a stale histogram, mixed levels and a refused value.

use super::{Levels, LevelsEditor};
use crate::ui::curve::stories::story_bins;
use crate::ui::histogram::{ChannelColor, HistogramData};
use craft_behavior::Mixed;
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::display::{Shade, Text};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use craft_ui::value_drag::Thumb;
use egui::Ui;

fn identity() -> Levels {
    Levels::identity(0.0..=255.0)
}

#[story(LevelsEditor<'static>, covers = [Default], drive = Drive::DragChild(Grip::part(Thumb::PART.at(0))))]
fn levels_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let bins = story_bins(0.0);
    let histogram = HistogramData::tone(&bins, ChannelColor::Neutral);
    let levels = data.value("levels", identity());
    let output = LevelsEditor::new("Levels", Bind::bound(&mut *levels)).histogram(&histogram).value("RGB channel").show(ui);
    let facts = output.into();
    let l = *levels;
    let readout = format!("Input {:.3} / {:.2} / {:.3}, output {:.3} / {:.3}", l.in_black, l.gamma, l.in_white, l.out_black, l.out_white);
    Text::new(readout).shade(Shade::Muted).show(ui);
    facts
}

#[story(LevelsEditor<'static>, covers = [], drive = Drive::Never)]
fn levels_editor_stale(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let bins = story_bins(12.0);
    let histogram = HistogramData::tone(&bins, ChannelColor::Red);
    LevelsEditor::new("Levels", Bind::bound(data.value("stale", identity())))
        .channel(ChannelColor::Red)
        .histogram(&histogram)
        .stale(true)
        .value("Red channel")
        .show(ui)
        .into()
}

#[story(LevelsEditor<'static>, covers = [Mixed], drive = Drive::Never)]
fn levels_editor_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    LevelsEditor::new("Levels", Bind::bound_mixed(data.value("mixed", Mixed::<Levels>::Mixed))).show(ui).into()
}

#[story(LevelsEditor<'static>, covers = [Invalid], drive = Drive::Never)]
fn levels_editor_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse = |levels: &Levels| {
        if levels.in_black > 100.0 { Validity::Refuse(Message::new("Input black is above the limit")) } else { Validity::Ok }
    };
    LevelsEditor::new("Levels", Bind::bound(data.value("over", Levels::new(150.0, 1.0, 255.0, 0.0, 255.0))).check(refuse)).show(ui).into()
}
