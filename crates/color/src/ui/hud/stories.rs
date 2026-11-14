//! Stories of `ColorHud`: open with a start colour, and with the pointer moved onto the strip.

use super::{ColorHud, strip_center};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Ui, pos2};

const START: Srgba = Srgba::rgb(0.4, 0.6, 0.8);

#[story(ColorHud<'static>, covers = [Default], drive = Drive::Never)]
fn color_hud(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    ColorHud::new("Color HUD", pos2(260.0, 200.0), Bind::given(START)).start(START).pointer(Some(pos2(260.0, 200.0))).show(ui).into()
}

#[story(ColorHud<'static>, covers = [], drive = Drive::Never)]
fn color_hud_pointer_on_the_strip(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let at = pos2(260.0, 200.0);
    let pointer = strip_center(&Env::of(ui).tokens, at, Some(START));
    ColorHud::new("Color HUD", at, Bind::given(START)).start(START).pointer(Some(pointer)).show(ui).into()
}
