//! Stories of the readout's container, shown through `ColorReadout`.

use craft_behavior::Mixed;
use craft_theme::Rgba;
use craft_ui::contract::{Bind, Validity};
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::ColorReadout;

#[story(ColorReadout<'static>, covers = [Default], drive = Drive::Never)]
fn color_readout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let color = data.value("color", Rgba::rgb(0x33, 0x66, 0x99));
    ColorReadout::new("Color", Bind::bound(color)).show(ui).into()
}

#[story(ColorReadout<'static>, covers = [], drive = Drive::Never)]
fn color_readout_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let color = data.value("mixed", Mixed::<Rgba>::Mixed);
    ColorReadout::new("Color", Bind::bound_mixed(color)).show(ui).into()
}

#[story(ColorReadout<'static>, covers = [Invalid], drive = Drive::Never)]
fn color_readout_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_dark = |color: &Rgba| if color.0[2] < 0x80 { Validity::Refuse(Message::new("Too dark")) } else { Validity::Ok };
    let color = data.value("dark", Rgba::rgb(0x33, 0x33, 0x33));
    ColorReadout::new("Color", Bind::bound(color).check(refuse_dark)).show(ui).into()
}
