//! Stories of `HexField`: a colour as hex digits, with two digits per channel, one (a web colour) and the alpha.

use super::HexField;
use crate::contract::{Bind, Show as _};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::text::TextField;
use craft_behavior::PlainText;
use craft_theme::Rgba;
use egui::Ui;

const BLUE: Rgba = Rgba([0x33, 0x66, 0x99, 0xff]);

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never, group = "Hex field")]
fn hex_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    HexField::hex("Hex color", Bind::bound(data.value("hex", BLUE))).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never, group = "Hex field")]
fn hex_field_web(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    HexField::hex("Web color", Bind::bound(data.value("hex", BLUE))).digits(1).show(ui).into()
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never, group = "Hex field")]
fn hex_field_alpha(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    HexField::hex("Hex color with alpha", Bind::bound(data.value("hex", Rgba([0x33, 0x66, 0x99, 0x80])))).alpha(true).show(ui).into()
}
