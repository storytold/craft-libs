//! `GradientOverlay` and the gradient line geometry, through the public API.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::CanvasView;
use craft_behavior::kurbo::Point;
use craft_draw::ui::gradient::gradient_line::{GradientLine, LineGradient};
use craft_draw::ui::gradient::{ColorStop, GradientOverlay, GradientStops, StopRef};
use craft_theme::Srgba;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

fn black() -> Srgba {
    Srgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
}

fn gradient() -> LineGradient {
    LineGradient::new(
        GradientLine::new(Point::new(20.0, 100.0), Point::new(220.0, 100.0)),
        GradientStops::new(vec![ColorStop::new(0.0, black()), ColorStop::new(0.5, black()), ColorStop::new(1.0, black())]),
    )
}

struct Shown {
    fixture: Fixture,
    selected: Rc<RefCell<Option<StopRef>>>,
}

fn shown() -> Shown {
    let held = Rc::new(RefCell::new(gradient()));
    let selected = Rc::new(RefCell::new(None));
    let chosen = Rc::clone(&selected);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        Canvas::new("Document").view(view).show(ui, |ui, body| {
            let mut value = held.borrow_mut();
            let mut selection = chosen.borrow_mut();
            GradientOverlay::new("Gradient", Bind::bound(&mut *value)).shared_selection(Bind::bound(&mut *selection)).show_at(ui, body.rect)
        });
    });
    fixture.settle();
    Shown { fixture, selected }
}

#[trial]
fn a_cancelled_press_on_a_stop_leaves_the_selection_as_it_was() {
    let mut s = shown();
    let stop = s.fixture.by_label("Color stop 2 of 3").expect("the overlay shows its stops as parts");
    s.fixture.press(&stop);
    assert_eq!(*s.selected.borrow(), Some(StopRef::Color(1)), "the press selects the stop");
    s.fixture.key(Key::Escape, Modifiers::NONE);
    s.fixture.release(&stop);
    s.fixture.settle();
    assert_eq!(*s.selected.borrow(), None);
}
