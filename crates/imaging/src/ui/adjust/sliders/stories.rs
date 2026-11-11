//! Stories of the slider table.

use super::{AdjustField, AdjustSliders, AdjustTrack};
use crate::ui::adjust::AdjustWords;
use crate::ui::adjust::style::{balance, mix, tones};
use craft_ui::contract::Bind;
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// Exposure's table: two places, four places and a short range.
#[story(AdjustSliders<'static>, covers = [Default], drive = Drive::Never)]
fn adjust_sliders(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let fields = AdjustField::exposure(&AdjustWords::default());
    let values = data.value("values", vec![0.0, 0.0, 1.0]);
    AdjustSliders::new("Exposure", &fields, Bind::bound(values)).show(ui).into()
}

/// A table with two-colour tracks.
#[story(AdjustSliders<'static>, covers = [], drive = Drive::Never)]
fn adjust_sliders_color_tracks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let fields = [
        AdjustField::new("cyanRed", "Cyan · Red", -100.0..=100.0, 0.0)
            .unit_kind(UnitKind::Percent)
            .track(AdjustTrack::Colors(balance::CYAN_RED.to_vec())),
        AdjustField::new("reds", "Reds", -200.0..=300.0, 40.0)
            .unit_kind(UnitKind::Percent)
            .track(AdjustTrack::Colors(vec![tones::BLACK, mix::RANGES[0]])),
    ];
    let values = data.value("colors", vec![0.0, 40.0]);
    AdjustSliders::new("Colors", &fields, Bind::bound(values)).show(ui).into()
}
