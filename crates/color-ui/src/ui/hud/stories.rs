//! Story of `ColorHud`: open with a start colour. The pick follows the runtime's pass pointer, which the HUD trials move.

use super::ColorHud;
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Ui, pos2};

const START: Srgba = Srgba::rgb(0.4, 0.6, 0.8);

#[story(ColorHud<'static>, covers = [Default], drive = Drive::Never)]
fn color_hud(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    ColorHud::new("Color HUD", pos2(260.0, 200.0), Bind::given(START)).start(START).show(ui).into()
}
