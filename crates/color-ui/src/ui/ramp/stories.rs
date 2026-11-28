//! Stories of `ColorRamp`: the spectrum, a grey ramp, a ramp between two colours and a web-safe one.

use super::{ColorRamp, RampKind, RampPick};
use craft_theme::Srgba;
use craft_ui::contract::{Props as _, Show as _, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

fn pick() -> RampPick {
    RampPick::new(Srgba::rgb(0.2, 0.5, 0.8), false)
}

#[story(ColorRamp<'static>, covers = [Default], drive = Drive::Drag(Grip::new(super::BAR)))]
fn color_ramp(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ColorRamp::new("Color ramp", RampKind::Spectrum, data.bind("pick", pick())).kind_name("RGB Spectrum").show(ui).into()
}

#[story(ColorRamp<'static>, covers = [], drive = Drive::Never)]
fn color_ramp_grayscale(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ColorRamp::new("Color ramp", RampKind::Grayscale, data.bind("pick", pick())).kind_name("Grayscale").show(ui).into()
}

#[story(ColorRamp<'static>, covers = [], drive = Drive::Never)]
fn color_ramp_between(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ColorRamp::new("Color ramp", RampKind::Between, data.bind("pick", pick()))
        .ends([Srgba::rgb(0.9, 0.2, 0.1), Srgba::rgb(0.1, 0.3, 0.9)])
        .kind_name("Foreground to background")
        .show(ui)
        .into()
}

#[story(ColorRamp<'static>, covers = [], drive = Drive::Never)]
fn color_ramp_web_safe(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ColorRamp::new("Color ramp", RampKind::Spectrum, data.bind("pick", pick())).web_safe(true).kind_name("Web colors").show(ui).into()
}

#[story(ColorRamp<'static>, covers = [], drive = Drive::Never)]
fn color_ramp_recolor_clicks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = ColorRamp::new("Color ramp", RampKind::Spectrum, data.bind("pick", pick())).recolor_clicks(true).kind_name("RGB Spectrum").show(ui);
    let last = data.note("last", String::new());
    if let Some(recolor) = shown.recolor {
        *last = format!("{recolor:?}");
    }
    let readout = format!("Last recolor: {last}");
    StoryOutput::from(shown).readout(readout)
}

#[story(ColorRamp<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_ramp_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dark = |pick: &RampPick| if pick.color.b < 0.5 { Validity::Refuse(Message::new("Too dark")) } else { Validity::Ok };
    let dark = RampPick::new(Srgba::rgb(0.1, 0.1, 0.1), false);
    ColorRamp::new("Color ramp", RampKind::Spectrum, data.bind("pick", dark).check(refuse_dark)).kind_name("RGB Spectrum").show(ui).into()
}

#[story(ColorRamp<'static>, covers = [], drive = Drive::Never, title = "Disabled")]
fn color_ramp_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ColorRamp::new("Color ramp", RampKind::Spectrum, data.bind("pick", pick())).kind_name("RGB Spectrum").enabled(false).show(ui).into()
}
