//! The arrow of a select.

use craft_ui::contract::{Bind, Entry, Show as _};
use craft_ui::pickers::Select;
use craft_ui::testing::{Fixture, trial};
use egui::Shape;

/// A select's arrow is a small filled triangle pointing down, as in a document app's choice boxes, not a stroked chevron.
#[trial]
fn a_select_ends_in_a_filled_down_triangle() {
    let rect = std::rc::Rc::new(std::cell::Cell::new(egui::Rect::NOTHING));
    let seen = rect.clone();
    let mut fixture = Fixture::new(move |ui| {
        let entries = [Entry::new(0u8, "One"), Entry::new(1u8, "Two")];
        seen.set(Select::new("Pick", Bind::given(1u8), &entries).show(ui).response.rect);
    });
    fixture.settle();
    let select = rect.get();
    let shapes = fixture.frame_shapes();
    let triangle = shapes.iter().any(|shape| match shape {
        Shape::Path(path) => path.points.len() == 3 && path.fill != egui::Color32::TRANSPARENT && select.contains_rect(path.visual_bounding_rect()),
        _ => false,
    });
    assert!(triangle, "the select paints a filled triangle for its arrow");
}
