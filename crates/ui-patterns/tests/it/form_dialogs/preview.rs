//! The preview's zoom row.

use craft_behavior::CanvasTransform;
use craft_ui::patterns::form_dialogs::{PreviewView, PreviewZoom};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// Zooms closer than this are the same zoom.
const SAME: f32 = 1e-4;

fn type_zoom(fixture: &mut Fixture, text: &str) {
    let field = fixture.by_label("Zoom").expect("the row has a zoom field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE).settle();
}

// A typed zoom sets the view's zoom, inside the view's zoom range.
#[trial]
fn a_typed_zoom_sets_the_preview_zoom_inside_its_range() {
    let view = Rc::new(RefCell::new(PreviewView::new(CanvasTransform::new(1.0, Vec2::ZERO))));
    let shown = view.clone();
    let mut fixture = Fixture::new(move |ui| {
        let current = shown.borrow().clone();
        if let Some(next) = PreviewZoom::new(&current).show(ui).view {
            *shown.borrow_mut() = next;
        }
    });
    fixture.settle();

    type_zoom(&mut fixture, "200");
    assert!((view.borrow().transform.scale - 2.0).abs() < SAME, "{:?}", view.borrow().transform);
    assert!(!view.borrow().fit);

    type_zoom(&mut fixture, "5000");
    let end = *view.borrow().range.end();
    assert!((view.borrow().transform.scale - end).abs() < SAME, "a zoom past the range stops at its end: {:?}", view.borrow().transform);
}
