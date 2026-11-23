//! Stories of the layout components `Row`, `Stack` and `Grid`.

use super::{Grid, Row, Stack};
use crate::contract::{Bind, Bracket, Gap, LeastWidth, Pad, Props as _, Show as _, ShowContainer as _, Sizing, WrapWidth};
use crate::display::Text;
use crate::form::{Field, FieldGroup};
use crate::layout::{CellAlign, GridFit, empty_cell};
use crate::story_kit::sample::{outlined, words};
use crate::testing::{StoryData, StoryOutput, story};
use crate::text::{NumberField, link_toggle};
use craft_behavior::ColumnWidth;
use egui::Ui;

/// The gap and padding tier the Row and Stack stories edit.
fn spacing(data: &mut StoryData, gap: Gap, gaps: &'static [(&'static str, Gap)]) -> (Gap, Pad) {
    let gap = *data.choice("gap", gap, gaps);
    let pad = *data.choice(
        "pad",
        Pad::Regular,
        &[("None", Pad::None), ("Control", Pad::Control), ("Regular", Pad::Regular), ("Roomy", Pad::Roomy), ("Page", Pad::Page)],
    );
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

/// A bracketed row: the width and height of a form linked to their chain, which sits halfway between the two fields'
/// controls. The bracket is drawn while they are linked.
#[story(Row, covers = [], title = "Linked rows")]
fn row_bracket(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let linked = *data.choice("linked", true, &[("Linked", true), ("Unlinked", false)]);
    outlined(ui, |ui| {
        FieldGroup::new("Size")
            .show(ui, |ui, _| {
                Row::new().bracket(if linked { Bracket::Linked } else { Bracket::Unlinked }).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        for name in ["Width", "Height"] {
                            Field::new(name).show(ui, |ui, _| {
                                NumberField::new(name, Bind::given(Some(800.0))).sizing(Sizing::Content).show(ui);
                            });
                        }
                    });
                    link_toggle("Constrain proportions", Bind::given(linked)).show(ui);
                });
            })
            .into()
    })
}

/// A side menu beside a page: the menu as wide as its widest item and as tall as the row, its footer at the bottom.
#[story(Stack<'static>, covers = [])]
fn stack_fill_height(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    outlined(ui, |ui| {
        // design: allow[contract/allocate, literal-size/vec2](the story gives the stack a fixed stage box to show fill-height sizing)
        ui.allocate_ui(egui::vec2(360.0, 200.0), |ui| {
            Row::new().fill().show(ui, |ui, _| {
                Stack::new()
                    .sizing(Sizing::FillHeight)
                    .gap(Gap::Tight)
                    .pad(Pad::Regular)
                    .footer(|ui, _| words(ui, 2..3))
                    .show(ui, |ui, _| words(ui, 0..2));
                Stack::new().fill().pad(Pad::Regular).show(ui, |ui, _| words(ui, 3..4));
            })
        })
        .inner
        .into()
    })
}

/// The two width roles of a content-sized stack: a side menu held at its least width with short items, and a column
/// of settings whose paragraph breaks at the column's line length instead of widening it. The column also holds a
/// dialog's least width, wider than its line length: where a paragraph's lines break moves with the text size, and the
/// least width keeps the story as wide in every density and text size, so it only grows as the text does. The two stand
/// one above the other: side by side they are wider than a trial's room, which then squeezes the column by as much as
/// a denser side menu grows, and its paragraph breaks narrower.
#[story(Stack<'static>, covers = [], title = "Width roles")]
fn stack_width_roles(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    outlined(ui, |ui| {
        Stack::new()
            .gap(Gap::Section)
            .show(ui, |ui, _| {
                Stack::new().least_width(LeastWidth::SideMenu).gap(Gap::Tight).pad(Pad::Regular).show(ui, |ui, _| words(ui, 0..2));
                let column = Stack::new().wrap_width(WrapWidth::Column).least_width(LeastWidth::Dialog);
                column.gap(Gap::Stack).pad(Pad::Regular).show(ui, |ui, _| {
                    let _ = Text::new(
                        "Save a copy in another format. Printing goes through a PDF you can print from any viewer, and the pages \
                         picked here are the ones that print.",
                    )
                    .show(ui);
                    words(ui, 2..3);
                });
            })
            .into()
    })
}

const COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Fixed(60.0)];

#[story(Grid, covers = [Default])]
fn grid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    outlined(ui, |ui| Grid::new(&COLUMNS).pad(Pad::Regular).show(ui, |ui, _| words(ui, 0..4)).into())
}

const FIXED: [ColumnWidth; 3] = [ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content];

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

const TOP: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::Content];

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
