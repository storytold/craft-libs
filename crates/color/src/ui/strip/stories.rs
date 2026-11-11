//! Stories of `ColorStrip`: a hue strip and a web-safe one.

use super::ColorStrip;
use crate::AreaMode;
use craft_theme::{ColorModel, HsbModel, Srgba};
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

#[story(ColorStrip<'static>, covers = [Default], drive = Drive::Step)]
fn color_strip(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorStrip::new("Hue", AreaMode::Hue, Bind::bound(values)).show(ui).into()
}

#[story(ColorStrip<'static>, covers = [], drive = Drive::Never)]
fn color_strip_web_safe(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.6, 0.8)));
    ColorStrip::new("Hue", AreaMode::Hue, Bind::bound(values)).quantize(6).show(ui).into()
}

#[story(ColorStrip<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_strip_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dim =
        |values: &craft_theme::ModelValues| if values.values[2] < 50.0 { Validity::Refuse(Message::new("Too dim")) } else { Validity::Ok };
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.1, 0.2, 0.3)));
    ColorStrip::new("Hue", AreaMode::Hue, Bind::bound(values).check(refuse_dim)).show(ui).into()
}
