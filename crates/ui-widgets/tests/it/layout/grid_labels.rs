//! The glyph labels beside the fields of one grid column share a width.

use craft_behavior::ColumnWidth;
use craft_ui::contract::{Bind, Entry, GlyphLabel, LabelKind, Show as _, ShowContainer as _, SideLabel};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Rect, Ui};
use std::cell::RefCell;
use std::rc::Rc;

/// A column sized by its cells, then one that fills the rest.
static COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::fill(1.0)];

/// The fields of each column of a grid start at one x, whatever the width of the glyph beside each: "I" and "W"
/// share the column's widest glyph. The narrow glyph comes first in the sized column, so its cell measures before the
/// column's widest glyph is known, and the column still holds both fields. Guards against a grid whose cells read no
/// shared label column, which set each field right after its own glyph.
#[trial]
fn the_fields_of_a_grid_column_share_a_left_edge() {
    let rects: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let seen = rects.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut seen = seen.borrow_mut();
        seen.clear();
        Grid::new(&COLUMNS).show(ui, |ui, _| {
            for (i, glyph) in ["I", "W", "W", "I"].into_iter().enumerate() {
                let side = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(glyph)));
                Field::new(format!("Field {i}")).side(side).show(ui, |ui, _| {
                    seen.push(NumberField::new(format!("Value {i}"), Bind::given(Some(10.0))).show(ui).response.rect);
                });
            }
        });
    });
    fixture.settle();
    let rects = rects.borrow();
    let [first, second, third, fourth] = rects[..] else { panic!("four fields show: {rects:?}") };
    assert!((first.left() - third.left()).abs() <= 0.5, "the sized column's fields share a left edge: {first:?}, {third:?}");
    assert!((second.left() - fourth.left()).abs() <= 0.5, "the filling column's fields share a left edge: {second:?}, {fourth:?}");
    assert!(first.right() <= second.left() && third.right() <= fourth.left(), "the sized column holds its fields: {rects:?}");
}

/// A filling column, then one sized by its cells.
static FILL_FIRST: [ColumnWidth; 2] = [ColumnWidth::fill(1.0), ColumnWidth::Content];

/// The rects of the two fields of a grid whose rows each hold a labelled field and, in the column sized by its cells,
/// text or a menu, in a room `width` wide; and the second field's label. `in_container`: the grid shows under a header
/// in a container that fills its width, beside menus, as a fixed section shows it.
fn grid_rows(in_container: bool, width: f32) -> ([Rect; 2], Rect) {
    let rects: Rc<RefCell<Vec<Rect>>> = Rc::default();
    let seen = rects.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut seen = seen.borrow_mut();
        seen.clear();
        let entries = [Entry::new(0u8, "Round"), Entry::new(1, "Inverted Round"), Entry::new(2, "Chamfer")];
        let mut grid = |ui: &mut Ui| {
            Grid::new(&FILL_FIRST).show(ui, |ui, _| {
                for i in 0..2 {
                    Field::new(format!("Top Left Corner {i}")).show(ui, |ui, _| {
                        seen.push(NumberField::new(format!("Radius {i}"), Bind::given(Some(10.0))).show(ui).response.rect);
                    });
                    if in_container {
                        Select::new(format!("Corner {i}"), Bind::given(0u8), &entries).show(ui);
                    } else {
                        Text::new("Round").show(ui);
                    }
                }
            });
        };
        if in_container {
            Stack::new()
                .fill_width()
                .header(|ui, _| {
                    Text::new("Rectangle").show(ui);
                })
                .show(ui, |ui, _| grid(ui));
        } else {
            grid(ui);
        }
    });
    fixture.room(egui::vec2(width, 400.0)).record_paint();
    fixture.settle();
    let rects = rects.borrow();
    let [first, second] = rects[..] else { panic!("two fields show: {rects:?}") };
    let label = fixture.texts().iter().find(|text| text.text == "Top Left Corner 1").map(|text| text.rect).expect("the second label shows");
    ([first, second], label)
}

/// A grid row is as tall as a field in it whose label sits beside its control or stacks above it for want of room:
/// the next row's label starts below the control, and the label shows whole. The grid stands alone beside text, and
/// fills a container's width beside menus at several widths. Guards against a grid that spaced its rows as if the
/// label sat beside the control and then placed it stacked, so the next row's label overlapped it; and against one
/// that took the least of a column of filling fields as none, so the field was left a sliver beside the menu.
#[trial]
fn a_grid_row_holds_a_field_whose_label_stacks() {
    let (_, whole) = grid_rows(false, 600.0);
    let ([first, second], label) = grid_rows(false, 150.0);
    assert!(label.bottom() <= second.top() + 0.5, "the label stacks above its control: {label:?}, {second:?}");
    assert!(label.top() >= first.bottom() - 0.5, "the second row starts below the first row's control: {label:?}, {first:?}");
    for width in [200.0, 240.0, 320.0] {
        let ([first, second], label) = grid_rows(true, width);
        assert!(label.top() >= first.bottom() - 0.5, "at {width}, the second row starts below the first row's control: {label:?}, {first:?}");
        assert!(second.top() >= first.bottom() - 0.5, "at {width}, the second field starts below the first: {second:?}, {first:?}");
        assert!(label.width() >= whole.width() - 0.5, "at {width}, the label shows whole: {label:?}, {whole:?}");
    }
}
