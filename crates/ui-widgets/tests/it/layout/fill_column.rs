//! A content-sized column holds its fill children and shows them across the width it settles on; a fill-width column
//! gives them its whole width.

use craft_ui::buttons::Button;
use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Divider, Text};
use craft_ui::layout_components::Stack;
use craft_ui::table::{MetadataGroup, MetadataRow, MetadataTable};
use craft_ui::testing::{Frames, trial};
use egui::Ui;
use std::cell::Cell;

const SNAP: f32 = 1.0;
const LONG: &str = "A heading as long as the column should settle on, wider than any control in it";

/// The widths of one column's stack, text, divider, table and button.
#[derive(Clone, Copy, Default)]
struct Widths {
    column: f32,
    text: f32,
    divider: f32,
    table: f32,
    button: f32,
}

fn column(ui: &mut Ui, fill_width: bool, groups: &[MetadataGroup], seen: &Cell<Widths>) {
    let stack = if fill_width { Stack::new().fill_width().id_salt("fill") } else { Stack::new().id_salt("hug") };
    let shown = stack.show(ui, |ui, _| {
        let text = Text::new(LONG).show(ui).rect.width();
        let divider = Divider::horizontal().show(ui).rect.width();
        let table = MetadataTable::new("Metadata", groups).show(ui).table.response.rect.width();
        let button = Button::new("Action").show(ui).rect.width();
        Widths { text, divider, table, button, ..Widths::default() }
    });
    if let Some(widths) = shown.inner {
        seen.set(Widths { column: shown.output.rect.width(), ..widths });
    }
}

fn widths(fill_width: bool) -> (Widths, f32) {
    let groups = vec![MetadataGroup::new("Camera", vec![MetadataRow::new("Exposure", "1/250 s"), MetadataRow::new("ISO", "200")])];
    let (seen, room) = (Cell::new(Widths::default()), Cell::new(0.0));
    let mut frames = Frames::new();
    frames.idle(4, |ui| {
        Stack::new().fill().show_at(ui, ui.available_rect_before_wrap(), |ui, _| {
            room.set(ui.available_width());
            column(ui, fill_width, &groups, &seen);
        });
    });
    (seen.get(), room.get())
}

#[trial]
fn a_content_column_shows_its_fill_children_across_its_width_and_a_fill_width_column_gives_them_the_room() {
    let (hug, _) = widths(false);
    assert!((hug.column - hug.text).abs() <= SNAP, "a content column is as wide as its widest content-sized child");
    assert!((hug.divider - hug.column).abs() <= SNAP, "a divider in a content column shows across it");
    assert!((hug.table - hug.column).abs() <= SNAP, "a table in a content column shows across it");
    assert!(hug.button < hug.column, "a button keeps its own width");

    let (fill, room) = widths(true);
    assert!((fill.divider - room).abs() <= SNAP, "a divider in a fill-width column takes the room");
    assert!((fill.table - room).abs() <= SNAP, "a table in a fill-width column takes the room");
    assert!((fill.button - hug.button).abs() <= SNAP, "a button keeps its own width in either column");
}
