//! Space is the canvas's Hand key, never a focused control's activation; Enter still activates.

use craft_behavior::Sense;
use craft_ui::testing::surface::root_widget;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn key(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

/// Space belongs to the canvas (its Hand tool), never to the control that holds the focus: egui would make it a click
/// on that control and drop the focus doing so, so a toolbar button or a panel row clicked before would take the key
/// instead of the canvas — and only sometimes, depending on what was clicked before. Enter keeps activating the
/// focused control, so every control stays reachable from the keyboard.
///
/// It drives `runtime::egui::response::widget`, the root widget every member resolves through, and fails while that
/// widget takes egui's Space-to-click default: the `InteractOptions` line is the whole fix.
#[allow(deprecated)]
#[trial]
fn space_does_not_activate_a_focused_control_and_enter_does() {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(120.0, 40.0));
    let clicked = Rc::new(RefCell::new(false));
    let id = Rc::new(Cell::new(None));
    let (seen, this) = (Rc::clone(&clicked), Rc::clone(&id));
    let mut fixture = Fixture::new(move |ui| {
        let control = Id::new("focused control");
        this.set(Some(control));
        let (was, now) = (*seen.borrow(), root_widget(ui, control, rect, Sense::CLICK));
        *seen.borrow_mut() = was || now;
    });
    fixture.settle();
    let control = id.get().expect("the control ran and reported its id");
    fixture.ctx().memory_mut(|memory| memory.request_focus(control));

    fixture.frame(vec![key(Key::Space)]);
    assert!(!*clicked.borrow(), "Space is the canvas's, not the control's activation");
    assert_eq!(fixture.ctx().memory(|memory| memory.focused()), Some(control), "the control keeps the focus");

    fixture.frame(vec![key(Key::Enter)]);
    assert!(*clicked.borrow(), "Enter still activates the focused control");
}
