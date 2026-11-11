//! Stories of the layout components `Row`, `Stack` and `Grid`.

use super::{Grid, Row, Stack};
use crate::contract::{Gap, Pad, ShowContainer as _};
use crate::story_kit::sample::{outlined, words};
use crate::testing::{StoryData, StoryOutput, story};
use craft_behavior::ColumnWidth;
use egui::Ui;

/// The gap and padding tier the Row and Stack stories edit.
fn spacing(data: &mut StoryData, gap: Gap, gaps: &'static [(&'static str, Gap)]) -> (Gap, Pad) {
    let gap = *data.choice("gap", gap, gaps);
    let pad = *data.choice("pad", Pad::Regular, &[("None", Pad::None), ("Control", Pad::Control), ("Regular", Pad::Regular), ("Roomy", Pad::Roomy)]);
    (gap, pad)
}

#[story(Row, covers = [Default])]
fn row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (gap, pad) = spacing(data, Gap::Row, &[("Row", Gap::Row), ("Tight", Gap::Tight), ("Section", Gap::Section)]);
    outlined(ui, |ui| Row::new().gap(gap).pad(pad).show(ui, |ui, _| words(ui, 0..3)).into())
}

#[story(Stack<'static>, covers = [Default])]
fn stack(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (gap, pad) = spacing(data, Gap::Stack, &[("Row", Gap::Row), ("Tight", Gap::Tight), ("Stack", Gap::Stack), ("Section", Gap::Section)]);
    outlined(ui, |ui| Stack::new().gap(gap).pad(pad).show(ui, |ui, _| words(ui, 0..3)).into())
}

#[story(Stack<'static>, covers = [])]
fn stack_footer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    outlined(ui, |ui| Stack::new().gap(Gap::Stack).pad(Pad::Regular).footer(|ui, _| words(ui, 2..3)).show(ui, |ui, _| words(ui, 0..2)).into())
}

static COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Fixed(60.0)];

#[story(Grid, covers = [Default])]
fn grid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    outlined(ui, |ui| Grid::new(&COLUMNS).pad(Pad::Regular).show(ui, |ui, _| words(ui, 0..4)).into())
}
