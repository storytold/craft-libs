//! Stories of `HexField`: a colour as hex digits after the "#" side label of its row, with two digits per channel, one
//! (a web colour) and the alpha.

use super::HexField;
use crate::contract::{Show as _, ShowContainer as _};
use crate::form::Field;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::text::TextField;
use craft_behavior::PlainText;
use craft_theme::Rgba;
use egui::Ui;

const BLUE: Rgba = Rgba([0x33, 0x66, 0x99, 0xff]);

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never, group = "Hex field")]
fn hex_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    in_row(ui, "Hex color", |ui| HexField::hex("Hex color", data.bind("hex", BLUE)).show(ui).into())
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never, group = "Hex field")]
fn hex_field_web(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    in_row(ui, "Web color", |ui| HexField::hex("Web color", data.bind("hex", BLUE)).digits(1).show(ui).into())
}

#[story(TextField<'static, PlainText>, covers = [], drive = Drive::Never, group = "Hex field")]
fn hex_field_alpha(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    in_row(ui, "Hex color with alpha", |ui| {
        HexField::hex("Hex color with alpha", data.bind("hex", Rgba([0x33, 0x66, 0x99, 0x80]))).alpha(true).show(ui).into()
    })
}

/// `field` in a row named `label` with the "#" side label every hex field shows before it.
fn in_row(ui: &mut Ui, label: &'static str, mut field: impl FnMut(&mut Ui) -> StoryOutput) -> StoryOutput {
    Field::new(label).side(HexField::hash_label()).show(ui, |ui, _| field(ui)).inner.unwrap_or_default()
}
