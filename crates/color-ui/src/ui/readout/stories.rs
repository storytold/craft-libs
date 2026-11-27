//! Stories of the readout's container, shown through `ColorReadout`.

use craft_behavior::Mixed;
use craft_theme::Rgba;
use craft_ui::contract::Validity;
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::ColorReadout;

#[story(ColorReadout<'static>, covers = [Default], drive = Drive::Never)]
fn color_readout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (color, color_at) = data.value_at("color", Rgba::rgb(0x33, 0x66, 0x99));
    ColorReadout::new("Color", color_at.bind(color)).show(ui).into()
}

#[story(ColorReadout<'static>, covers = [], drive = Drive::Never)]
fn color_readout_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (color, color_at) = data.value_at("mixed", Mixed::<Rgba>::Mixed);
    ColorReadout::new("Color", color_at.bind_mixed(color)).show(ui).into()
}

#[story(ColorReadout<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_readout_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dark = |color: &Rgba| if color.0[2] < 0x80 { Validity::Refuse(Message::new("Too dark")) } else { Validity::Ok };
    let (color, color_at) = data.value_at("dark", Rgba::rgb(0x33, 0x33, 0x33));
    ColorReadout::new("Color", color_at.bind(color).check(refuse_dark)).show(ui).into()
}
