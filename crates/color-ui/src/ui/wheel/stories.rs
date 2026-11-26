//! Stories of `ColorWheel`.

use super::{ColorWheel, Gamut, WheelInner};
use craft_theme::{ColorModel, Hsb, HsbModel, Srgba};
use craft_ui::contract::{Props as _, Show as _, Validity};
use craft_ui::runtime::env::{Label, Message};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

const MARKERS: [Srgba; 3] = [Srgba::rgb(0.9, 0.2, 0.2), Srgba::rgb(0.2, 0.7, 0.4), Srgba::rgb(0.3, 0.4, 0.9)];

#[story(ColorWheel<'static>, covers = [Default], drive = Drive::Drag(Grip::new(super::RING)))]
fn color_wheel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    let shown = ColorWheel::new("Color wheel", values_at.bind(values)).show(ui);
    let last = data.note("last", String::new());
    if let Some(event) = shown.edit.events.last() {
        *last = format!("{event:?}");
    }
    let readout = format!("Last event: {last}");
    StoryOutput::from(shown).readout(readout)
}

#[story(ColorWheel<'static>, covers = [], drive = Drive::Drag(Grip::new(super::RING)), title = "Square")]
fn color_wheel_square(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ColorWheel::new("Color wheel", data.bind("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)))).inner(WheelInner::Square).show(ui).into()
}

#[story(ColorWheel<'static>, covers = [], drive = Drive::Never, title = "Square with markers")]
fn color_wheel_square_markers(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let names = ["Warm", "Green", "Cool"].map(|name| Label::from(name.to_owned())).to_vec();
    let values = data.bind("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    let shown = ColorWheel::new("Color wheel", values).inner(WheelInner::Square).markers(&MARKERS).marker_names(names).show(ui);
    let picked = data.note("picked", String::new());
    if let Some(index) = shown.picked {
        *picked = format!("Picked: {index}");
    }
    let readout = picked.clone();
    StoryOutput::from(shown).readout(readout)
}

#[story(ColorWheel<'static>, covers = [], drive = Drive::Never, title = "Square with a gamut")]
fn color_wheel_square_gamut(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    // A gamut that cannot reach full saturation: the square shades what it cannot show.
    let gamut = Gamut::new(|color| {
        let hsb = color.to_hsb();
        Srgba::from_hsb(Hsb { s: hsb.s.min(0.6), ..hsb })
    });
    let values = data.bind("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorWheel::new("Color wheel", values).inner(WheelInner::Square).gamut(gamut).show(ui).into()
}

#[story(ColorWheel<'static>, covers = [Mixed], drive = Drive::Never, title = "Square, mixed")]
fn color_wheel_square_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let values = data.bind_mixed("mixed", craft_behavior::Mixed::<craft_theme::ModelValues>::Mixed);
    ColorWheel::new("Color wheel", values).inner(WheelInner::Square).show(ui).into()
}

#[story(ColorWheel<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_wheel_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dim =
        |values: &craft_theme::ModelValues| if values.values[2] < 50.0 { Validity::Refuse(Message::new("Too dim")) } else { Validity::Ok };
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.1, 0.2, 0.3)));
    ColorWheel::new("Color wheel", values_at.bind(values).check(refuse_dim)).show(ui).into()
}

#[story(ColorWheel<'static>, covers = [], drive = Drive::Never, title = "Disabled")]
fn color_wheel_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.2, 0.5, 0.8)));
    ColorWheel::new("Color wheel", values_at.bind(values)).enabled(false).show(ui).into()
}
