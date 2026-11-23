//! The glyph labels beside the fields of one grid column share a width.

use craft_behavior::ColumnWidth;
use craft_ui::contract::{Bind, GlyphLabel, LabelKind, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::layout_components::Grid;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::Rect;
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
