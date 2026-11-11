//! Stories of `ColorArea`: a hue-mode area and a web-safe one.

use super::ColorArea;
use crate::AreaMode;
use craft_theme::{ColorModel, HsbModel, Srgba};
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use craft_ui::value_drag::PointPicker;
use egui::Ui;

/// The value changes through the drag of the point picker the area shows inside itself.
#[story(ColorArea<'static>, covers = [Default], drive = Drive::DragChild(Grip::new(PointPicker::WELL)))]
fn color_area(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, Bind::bound(values)).show(ui).into()
}

#[story(ColorArea<'static>, covers = [], drive = Drive::Never)]
fn color_area_web_safe(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.6, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, Bind::bound(values)).quantize(6).show(ui).into()
}

#[story(ColorArea<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_area_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dim =
        |values: &craft_theme::ModelValues| if values.values[2] < 50.0 { Validity::Refuse(Message::new("Too dim")) } else { Validity::Ok };
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.1, 0.2, 0.3)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, Bind::bound(values).check(refuse_dim)).show(ui).into()
}
