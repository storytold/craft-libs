//! Stories of the layout components `Row`, `Stack` and `Grid`.

use super::{Grid, Row, Stack};
use crate::contract::{Gap, Pad, ShowContainer as _};
use crate::layout::{CellAlign, GridFit, empty_cell};
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

static FIXED: [ColumnWidth; 3] = [ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content];

/// A grid whose arrangement is fixed: in a room narrower than its columns it keeps them beside each other and
/// overflows, and its second row leaves the first cell empty, so the cells after it stay in their columns.
#[story(Grid, covers = [])]
fn grid_fixed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    outlined(ui, |ui| {
        Grid::new(&FIXED)
            .fit(GridFit::Fixed)
            .pad(Pad::Regular)
            .show(ui, |ui, _| {
                words(ui, 0..3);
                empty_cell(ui);
                words(ui, 2..4);
            })
            .into()
    })
}

static TOP: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Content];

/// A grid whose cells stand at the top of their row: a short column of words beside a taller one starts level with it.
#[story(Grid, covers = [])]
fn grid_cells_top(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    outlined(ui, |ui| {
        Grid::new(&TOP)
            .cells(CellAlign::Top)
            .pad(Pad::Regular)
            .show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| words(ui, 0..1));
                Stack::new().show(ui, |ui, _| words(ui, 0..4));
            })
            .into()
    })
}
