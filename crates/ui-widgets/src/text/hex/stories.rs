//! Stories of `HexField`.

use super::HexField;
use crate::contract::{Bind, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::text::TextField;
use craft_behavior::PlainText;
use craft_theme::Rgba;
use egui::Ui;

const ORANGE: Rgba = Rgba([0xff, 0x88, 0x00, 0xff]);

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never)]
fn hex_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    HexField::hex("Hex color", Bind::bound(data.value("hex", ORANGE))).show(ui).into()
}
