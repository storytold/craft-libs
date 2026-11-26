//! Stories of `AxisSliders`: the Weight and Width axes of a variable face with three named instances.

use craft_ui::testing::{Drive, StoryData, StoryOutput, ValueAt, story};
use egui::Ui;

use super::{AxisSliders, AxisValues, OpticalSize};
use crate::axes::{FontAxis, NamedInstance};

const WGHT: [u8; 4] = *b"wght";
const WDTH: [u8; 4] = *b"wdth";

#[story(AxisSliders<'static>, covers = [Default], drive = Drive::Never)]
fn axis_sliders(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let axes = [
        FontAxis { tag: WGHT, name: "Weight".to_owned(), min: 100.0, default: 400.0, max: 900.0, hidden: false },
        FontAxis { tag: WDTH, name: "Width".to_owned(), min: 75.0, default: 100.0, max: 100.0, hidden: false },
    ];
    let instance = |name: &str, weight: f32, width: f32| NamedInstance { name: name.to_owned(), coordinates: vec![(WGHT, weight), (WDTH, width)] };
    let instances = [instance("Light", 300.0, 100.0), instance("Regular", 400.0, 100.0), instance("Bold Condensed", 700.0, 75.0)];
    let (values, values_at): (&mut AxisValues, ValueAt<'_>) = data.value_at("values", vec![(WGHT, 400.0), (WDTH, 100.0)]);
    AxisSliders::new("Axes", &axes, values_at.bind(values)).instances(&instances).show(ui).into()
}

/// Sliders that also set the optical size, for text drawn only to look at the face.
#[story(AxisSliders<'static>, covers = [], drive = Drive::Never)]
fn axis_sliders_optical_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let axes = [FontAxis::new(WGHT, "", 100.0, 400.0, 900.0), FontAxis::new(*b"opsz", "", 8.0, 14.0, 144.0)];
    let (values, values_at): (&mut AxisValues, ValueAt<'_>) = data.value_at("values", vec![(WGHT, 400.0)]);
    AxisSliders::new("Axes", &axes, values_at.bind(values)).optical_size(OpticalSize::Slider).show(ui).into()
}
