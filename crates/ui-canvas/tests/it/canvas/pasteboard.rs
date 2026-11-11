//! `Pasteboard`: the Menu key or Shift+F10 on the focused canvas asks for the pasteboard menu at the view's centre, as
//! a secondary click on the pasteboard does where it lands.

use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect};
use craft_ui::canvas::{Canvas, PageFrame, Pasteboard};
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

#[trial]
fn the_menu_key_on_the_focused_canvas_asks_for_the_pasteboard_menu_at_the_views_centre() {
    let centre = Point::new(200.0, 150.0);
    let asked: Rc<Cell<Option<Point>>> = Rc::new(Cell::new(None));
    let seen = Rc::clone(&asked);
    let pages = [PageFrame::new(Rect::new(100.0, 80.0, 300.0, 220.0)).name("Artboard 1")];
    let mut fixture = Fixture::new(move |ui| {
        let view = CanvasView::new(ui.available_rect_before_wrap(), centre, 1.0);
        let shown = Canvas::new("Document").view(view).show(ui, |ui, body| Pasteboard::new("Pasteboard", &pages).show_at(ui, body.rect));
        if let Some(at) = shown.inner.and_then(|out| out.context_at) {
            seen.set(Some(at));
        }
    });
    fixture.settle().key(Key::Tab, Modifiers::NONE).settle();
    fixture.key(Key::F10, Modifiers::SHIFT).settle();
    // The centre lies on the page: a keyed request has no point of its own to test against the pages.
    assert_eq!(asked.get(), Some(centre));
}
