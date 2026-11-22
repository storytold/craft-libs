//! Stories of `SwatchPalette`.

use super::SwatchPalette;
use craft_theme::Srgba;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The twelve colours of the palette story.
const PALETTE: [(&str, Srgba); 12] = [
    ("Red", Srgba::rgb(0.90, 0.28, 0.30)),
    ("Orange", Srgba::rgb(0.97, 0.42, 0.08)),
    ("Amber", Srgba::rgb(0.96, 0.65, 0.14)),
    ("Yellow", Srgba::rgb(0.96, 0.85, 0.04)),
    ("Lime", Srgba::rgb(0.48, 0.77, 0.36)),
    ("Green", Srgba::rgb(0.19, 0.64, 0.42)),
    ("Teal", Srgba::rgb(0.07, 0.65, 0.58)),
    ("Cyan", Srgba::rgb(0.00, 0.62, 0.88)),
    ("Blue", Srgba::rgb(0.24, 0.39, 0.87)),
    ("Violet", Srgba::rgb(0.56, 0.31, 0.78)),
    ("Magenta", Srgba::rgb(0.84, 0.25, 0.62)),
    ("Grey", Srgba::rgb(0.53, 0.55, 0.57)),
];

fn named(entries: impl IntoIterator<Item = (String, Srgba)>) -> Vec<(Label, Srgba)> {
    entries.into_iter().map(|(name, color)| (Label::from(name), color)).collect()
}

#[story(SwatchPalette<'static>, covers = [], drive = Drive::Never)]
fn swatch_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let swatches = named(PALETTE.iter().map(|(name, color)| ((*name).to_owned(), *color)));
    let shown = SwatchPalette::new("Swatches", data.bind("color", PALETTE[8].1), &swatches).show(ui);
    let last = data.note("last", String::new());
    if let Some(event) = &shown.edit {
        *last = format!("{event:?}");
    }
    let readout = format!("Last event: {last}");
    StoryOutput::from(shown).readout(readout)
}
