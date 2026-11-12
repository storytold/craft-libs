//! The Navigator's preview keys: Home centres the document.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Size};
use craft_ui::canvas::Navigator;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Rect, pos2, vec2};
use std::cell::Cell;
use std::rc::Rc;

const DOCUMENT: Size = Size::new(2000.0, 1500.0);

#[trial]
fn home_on_the_focused_preview_centres_the_document_and_end_leaves_the_view() {
    let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let view = Rc::new(Cell::new(CanvasView::new(canvas, Point::new(400.0, 300.0), 1.0)));
    let shown = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let mut given = shown.get();
        let _ = Navigator::new("Navigator", Bind::bound(&mut given), DOCUMENT).show(ui);
        shown.set(given);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    assert!(fixture.focused_stop().is_some(), "Tab reaches the preview");
    let before = view.get().center;
    fixture.key(Key::End, Modifiers::NONE);
    assert_eq!(view.get().center, before, "End leaves the view where it is");
    fixture.key(Key::Home, Modifiers::NONE);
    assert_eq!(view.get().center, Point::new(DOCUMENT.width / 2.0, DOCUMENT.height / 2.0), "Home centres the document");
}
