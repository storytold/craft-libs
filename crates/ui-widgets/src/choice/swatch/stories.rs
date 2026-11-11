//! Stories of `Swatch`.

use super::Swatch;
use crate::choice::RadioGroup;
use crate::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::layout_components::{Row, Stack};
use crate::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use craft_behavior::Mixed;
use craft_theme::Srgba;
use egui::Ui;

const BLUE: Srgba = Srgba::rgb(0.24, 0.39, 0.87);

/// The twelve colours of the palette story.
const PALETTE: [Srgba; 12] = [
    Srgba::rgb(0.90, 0.28, 0.30),
    Srgba::rgb(0.97, 0.42, 0.08),
    Srgba::rgb(0.96, 0.65, 0.14),
    Srgba::rgb(0.96, 0.85, 0.04),
    Srgba::rgb(0.48, 0.77, 0.36),
    Srgba::rgb(0.19, 0.64, 0.42),
    Srgba::rgb(0.07, 0.65, 0.58),
    Srgba::rgb(0.00, 0.62, 0.88),
    Srgba::rgb(0.24, 0.39, 0.87),
    Srgba::rgb(0.56, 0.31, 0.78),
    Srgba::rgb(0.84, 0.25, 0.62),
    Srgba::rgb(0.53, 0.55, 0.57),
];

#[story(Swatch<u8>, covers = [Default])]
fn swatch(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Swatch::new("Fill", BLUE).show(ui).into()
}

/// Opaque, translucent, clear, a colour equal to the panel, mixed and selected, each named by its label.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_states(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Row::new().show(ui, |ui, _| {
        Swatch::new("Opaque", BLUE).show(ui);
        Swatch::new("Translucent", Srgba { r: 0.90, g: 0.28, b: 0.30, a: 0.43 }).show(ui);
        Swatch::new("Clear", Srgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }).show(ui);
        Swatch::new("Mixed", Mixed::<Srgba>::Mixed).show(ui);
        Swatch::new("Selected", PALETTE[5]).selected(true).show(ui);
    });
    StoryOutput::default()
}

/// Twelve swatches in a `RadioGroup`: one Tab stop, arrows move, Space picks.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    RadioGroup::new("Palette", Bind::bound(data.value("palette", 8u8)))
        .show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                for (n, color) in (0u8..).zip(PALETTE) {
                    Swatch::new(format!("Colour {}", n + 1), color).member(n).show(ui);
                }
            });
        })
        .into()
}

/// A row of `cell` swatches, one selected: the owning table places them against each other.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_table_cells(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Row::new().gap(Gap::Tight).show(ui, |ui, _| {
        for (n, color) in PALETTE.into_iter().take(6).enumerate() {
            // A cell's place in its set is the owning table's item step; each story cell stands alone in its own.
            Row::new().show(ui, |ui, _| {
                show_as_only_item(ui, Swatch::new(format!("Cell {}", n + 1), color).cell().selected(n == 2));
            });
        }
    });
    StoryOutput::default()
}

/// A well and a line that counts its clicks.
#[story(Swatch<u8>, covers = [], drive = Drive::Never)]
fn swatch_clicks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let clicks = data.value("clicks", 0u32);
    Stack::new().show(ui, |ui, _| {
        if Swatch::new("Fill", BLUE).show(ui).clicked {
            *clicks += 1;
        }
        Text::new(format!("{clicks} clicks")).show(ui);
    });
    StoryOutput::default()
}
