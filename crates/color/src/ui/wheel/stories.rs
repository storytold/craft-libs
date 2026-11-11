//! Stories of `ColorWheel`.

use super::ColorWheel;
use craft_theme::{ColorModel, HsbModel, Srgba};
use craft_ui::contract::{Bind, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

#[story(ColorWheel<'static>, covers = [Default], drive = Drive::Drag(Grip::new(super::RING)))]
fn color_wheel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorWheel::new("Color wheel", Bind::bound(values)).show(ui).into()
}

#[story(ColorWheel<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_wheel_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dim =
        |values: &craft_theme::ModelValues| if values.values[2] < 50.0 { Validity::Refuse(Message::new("Too dim")) } else { Validity::Ok };
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.1, 0.2, 0.3)));
    ColorWheel::new("Color wheel", Bind::bound(values).check(refuse_dim)).show(ui).into()
}
