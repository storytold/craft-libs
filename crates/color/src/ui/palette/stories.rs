//! Stories of `SwatchPalette` and `SwatchTable`.

use super::{SwatchPalette, SwatchTable, SwatchTableOutput};
use craft_behavior::Selection;
use craft_theme::Srgba;
use craft_ui::contract::Bind;
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

/// A 256-entry table whose entries repeat colours: index 3 and 7 share one.
fn table_entries() -> Vec<(Label, Srgba)> {
    named((0..256usize).map(|i| {
        let shade = if i == 7 { 3 } else { i };
        (format!("Index {i}"), Srgba::rgb(shade as f32 / 255.0, (shade % 16) as f32 / 15.0, 0.5))
    }))
}

/// The table's output with the readout of its latest select or open, kept in the story's data across passes.
fn with_readout(data: &mut StoryData, shown: SwatchTableOutput) -> StoryOutput {
    let last = data.note("last", String::new());
    if let Some(index) = shown.selected {
        *last = format!("select {index}");
    }
    if let Some(index) = shown.open {
        *last = format!("open {index}");
    }
    let readout = format!("Last event: {last}");
    StoryOutput::from(shown).readout(readout)
}

#[story(SwatchPalette<'static>, covers = [], drive = Drive::Never)]
fn swatch_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let swatches = named(PALETTE.iter().map(|(name, color)| ((*name).to_owned(), *color)));
    let shown = SwatchPalette::new("Swatches", Bind::bound(data.value("color", PALETTE[8].1)), &swatches).show(ui);
    let last = data.note("last", String::new());
    if let Some(event) = &shown.edit {
        *last = format!("{event:?}");
    }
    let readout = format!("Last event: {last}");
    StoryOutput::from(shown).readout(readout)
}

#[story(SwatchTable<'static>, covers = [Default, Many], drive = Drive::Never)]
fn swatch_table(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let shown =
        SwatchTable::new("Color Table", &entries).selection(Bind::bound(data.value("selected", Selection::single(5usize)))).per_row(16).show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_multiple(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let start = Selection::new(vec![2usize, 3, 18], Some(18), Some(2));
    let shown =
        SwatchTable::new("Color Table", &entries).multiple(true).selection(Bind::bound(data.value("selected", start))).per_row(16).show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [Empty], drive = Drive::Never)]
fn swatch_table_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SwatchTable::new("Color Table", &[]).empty("No colors").show_all(ui).into()
}

#[story(SwatchTable<'static>, covers = [One], drive = Drive::Never)]
fn swatch_table_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries().into_iter().take(1).collect::<Vec<_>>();
    let shown = SwatchTable::new("Color Table", &entries).selection(Bind::bound(data.value("selected", Selection::single(0usize)))).show_all(ui);
    with_readout(data, shown)
}
