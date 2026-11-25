//! Stories of `ColorArea`: its two shapes, a web-safe area, a refused value and a disabled one.

use super::{AreaShape, ColorArea};
use crate::AreaMode;
use crate::ui::Gamut;
use craft_theme::{ColorModel, HsbModel, Srgba};
use craft_ui::contract::{Props as _, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use craft_ui::value_drag::PointPicker;
use egui::Ui;

/// The compact pickers' field at plot proportions. The value changes through the drag of the point picker the area
/// shows inside itself.
#[story(ColorArea<'static>, covers = [Default], drive = Drive::DragChild(Grip::new(PointPicker::WELL)))]
fn color_area(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values_at.bind(values)).show(ui).into()
}

/// The Color Picker dialog's field: a square, `size.plot.width` on a side.
#[story(ColorArea<'static>, covers = [], drive = Drive::Never)]
fn color_area_square(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values_at.bind(values)).shape(AreaShape::Square).show(ui).into()
}

#[story(ColorArea<'static>, covers = [], drive = Drive::Never)]
fn color_area_web_safe(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.6, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values_at.bind(values)).quantize(6).show(ui).into()
}

#[story(ColorArea<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_area_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dim =
        |values: &craft_theme::ModelValues| if values.values[2] < 50.0 { Validity::Refuse(Message::new("Too dim")) } else { Validity::Ok };
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.1, 0.2, 0.3)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values_at.bind(values).check(refuse_dim)).show(ui).into()
}

#[story(ColorArea<'static>, covers = [], drive = Drive::Never)]
fn color_area_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values_at.bind(values)).enabled(false).show(ui).into()
}

/// A gamut that moves the pale colours (the top-left quarter of the hue field) to black: the cells it moves are shaded
/// and edged.
#[story(ColorArea<'static>, covers = [], drive = Drive::Never)]
fn color_area_gamut(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.bind("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    let pale_to_black = Gamut::new(|color: Srgba| {
        let (high, low) = (color.r.max(color.g).max(color.b), color.r.min(color.g).min(color.b));
        if high > 0.5 && low > high / 2.0 { Srgba::rgb(0.0, 0.0, 0.0) } else { color }
    });
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values).shape(AreaShape::Square).gamut(pale_to_black).show(ui).into()
}
