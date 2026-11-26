//! Stories of `InfoPanel`: a full reading with two samplers, a protractor's measure, readings with every value missing,
//! and no document.

use super::{InfoColor, InfoMeasure, InfoPanel, InfoReadings};
use crate::ui::SamplerReadout;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn pair(x: &'static str, y: &'static str) -> [Label; 2] {
    [Label::from(x), Label::from(y)]
}

#[story(InfoPanel<'static>, covers = [Default], drive = Drive::Never)]
fn info_panel(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let samplers = [
        (SamplerReadout::from_values(1, [120, 80], [255, 128, 0], [0, 50, 100, 0]), pair("120", "80")),
        (SamplerReadout::from_values(2, [300, 41], [10, 20, 30], [67, 33, 0, 88]), pair("300", "41")),
    ];
    let readings = InfoReadings::new()
        .color(Some(InfoColor { rgb: [64, 128, 192], cmyk: [67, 33, 0, 25] }))
        .position(Some(pair("512", "384")))
        .extent(Some(pair("200", "100")))
        .document(Some([1920, 1080]))
        .samplers(&samplers);
    InfoPanel::new("Info", &readings).show(ui).into()
}

#[story(InfoPanel<'static>, covers = [], drive = Drive::Never)]
fn info_panel_missing_values(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let readings = InfoReadings::new().document(Some([1920, 1080]));
    InfoPanel::new("Info", &readings).show(ui).into()
}

#[story(InfoPanel<'static>, covers = [], drive = Drive::Never)]
fn info_panel_no_document(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let readings = InfoReadings::new();
    InfoPanel::new("Info", &readings).show(ui).into()
}

#[story(InfoPanel<'static>, covers = [], drive = Drive::Never)]
fn info_panel_protractor(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let readings = InfoReadings::new()
        .color(Some(InfoColor { rgb: [64, 128, 192], cmyk: [67, 33, 0, 25] }))
        .measure(Some(InfoMeasure::protractor("45.0°", "200.00", "141.42")))
        .position(Some(pair("100", "80")))
        .extent(Some(pair("200", "0")))
        .document(Some([1920, 1080]));
    InfoPanel::new("Info", &readings).show(ui).into()
}
