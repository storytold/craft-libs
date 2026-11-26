//! Stories of `SwatchTable`.

use super::{CellMarks, SwatchArrangement, SwatchSpacing, SwatchTable, SwatchTableOutput};
use crate::contract::ActivateOn;
use crate::runtime::env::Label;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Preview, Selection};
use craft_theme::Srgba;
use egui::Ui;

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

#[story(SwatchTable<'static>, covers = [Default, Many], drive = Drive::Never)]
fn swatch_table(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let shown = SwatchTable::new("Color Table", &entries).selection(data.bind("selected", Selection::single(5usize))).per_row(16).show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_multiple(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let start = Selection::new(vec![2usize, 3, 18], Some(18), Some(2));
    let shown = SwatchTable::new("Color Table", &entries).multiple(true).selection(data.bind("selected", start)).per_row(16).show_all(ui);
    with_readout(data, shown)
}

/// The readout follows the preview: the cell under the pointer while it shows, the selection once it ends.
#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_preview(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries().into_iter().take(64).collect::<Vec<_>>();
    let shown = SwatchTable::new("Color Table", &entries).selection(data.bind("selected", Selection::single(5usize))).per_row(16).show_all(ui);
    let previewed = data.note("previewed", None::<usize>);
    match shown.preview {
        Preview::Shows(index) => *previewed = Some(index),
        Preview::Ended => *previewed = None,
        _ => {}
    }
    let readout = match *previewed {
        Some(index) => format!("Previewing: Index {index}"),
        None => "Previewing: nothing".to_owned(),
    };
    StoryOutput::from(shown).readout(readout)
}

/// 127 entries, the full honeycomb of side 7.
#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_honeycomb(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries().into_iter().take(127).collect::<Vec<_>>();
    let shown = SwatchTable::new("Color Table", &entries)
        .arrangement(SwatchArrangement::Honeycomb)
        .selection(data.bind("selected", Selection::single(5usize)))
        .show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_spaced(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let shown = SwatchTable::new("Color Table", &entries)
        .spacing(SwatchSpacing::Spaced)
        .selection(data.bind("selected", Selection::single(5usize)))
        .per_row(16)
        .show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_spaced_draggable(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let shown = SwatchTable::new("Color Table", &entries)
        .spacing(SwatchSpacing::Spaced)
        .drag_payload(|_| Some(String::from("swatch")))
        .selection(data.bind("selected", Selection::single(5usize)))
        .per_row(16)
        .show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_marks(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let start = Selection::new(vec![2usize, 3], Some(3), Some(2));
    let shown = SwatchTable::new("Color Table", &entries)
        .multiple(true)
        .marks(|&at| match at {
            1 => CellMarks::default().edge(true),
            3 => CellMarks::default().locked(true),
            5 => CellMarks::default().web_safe(true),
            7 => CellMarks::default().transparent(true),
            9 => CellMarks::default().source(Srgba::rgb(0.9, 0.2, 0.1)),
            _ => CellMarks::default(),
        })
        .selection(data.bind("selected", start))
        .per_row(16)
        .show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_one_row_click(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries();
    let shown = SwatchTable::new("Color Table", &entries)
        .max_rows(1)
        .activate(ActivateOn::Click)
        .selection(data.bind("selected", Selection::single(5usize)))
        .show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [Empty], drive = Drive::Never)]
fn swatch_table_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    SwatchTable::new("Color Table", &[]).empty("No colors").show_all(ui).into()
}

/// Twelve wells and the Other… cell after them, at key 12: selecting or opening it reports that key.
#[story(SwatchTable<'static>, covers = [], drive = Drive::Never)]
fn swatch_table_trailing(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries().into_iter().step_by(21).take(12).collect::<Vec<_>>();
    let shown = SwatchTable::new("Colors", &entries)
        .selection(data.bind("selected", Selection::single(0usize)))
        .wells(true)
        .tips(true)
        .per_row(6)
        .trailing("Other colour…", IconId::new("plus"))
        .show_all(ui);
    with_readout(data, shown)
}

#[story(SwatchTable<'static>, covers = [One], drive = Drive::Never)]
fn swatch_table_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entries = table_entries().into_iter().take(1).collect::<Vec<_>>();
    let shown = SwatchTable::new("Color Table", &entries).selection(data.bind("selected", Selection::single(0usize))).show_all(ui);
    with_readout(data, shown)
}
