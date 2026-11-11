//! A viewport window blocks nothing unless it is app-modal, and only an app-modal window closes on Escape.

use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::overlay::{Backdrop, CloseReason};
use craft_ui::shell::ViewportWindow;
use craft_ui::testing::contract::clicked;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// Presses on the button under the window, where it is, and the window's last close request.
#[derive(Default)]
struct Seen {
    presses: Cell<u32>,
    under: Cell<Option<egui::Rect>>,
    close: Cell<Option<CloseReason>>,
}

fn rig(modal: Option<Backdrop>) -> (Fixture, Rc<Seen>) {
    let seen = Rc::new(Seen::default());
    let sink = seen.clone();
    let mut fixture = Fixture::new(move |ui| {
        let under = Button::new("Under").show(ui);
        sink.under.set(Some(under.response.rect));
        if clicked(&under) {
            sink.presses.set(sink.presses.get() + 1);
        }
        let mut window = ViewportWindow::new("Log");
        if let Some(backdrop) = modal {
            window = window.app_modal(backdrop);
        }
        let mut on = false;
        let shown = window.show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut on)).label("Wrap").show(ui);
        });
        if shown.close_requested.is_some() {
            sink.close.set(shown.close_requested);
        }
    });
    fixture.idle(30);
    (fixture, seen)
}

/// Clicks the button under the window, then focuses the window's check box and presses Escape. The press goes to the
/// button's position: a modal layer hides what is under it from the accessibility tree.
fn press_under_then_escape(fixture: &mut Fixture, seen: &Seen) {
    let under = seen.under.get().expect("the button under the window is shown");
    fixture.click_at(under.center()).idle(4);
    let wrap = fixture.by_label("Wrap").expect("the window's body is shown");
    fixture.click(&wrap).idle(4);
    fixture.key(Key::Escape, Modifiers::NONE).idle(4);
}

#[trial]
fn a_tool_window_blocks_nothing_and_ignores_escape() {
    let (mut fixture, seen) = rig(None);
    press_under_then_escape(&mut fixture, &seen);
    assert_eq!(seen.presses.get(), 1, "the press reached the window under it");
    assert_eq!(seen.close.get(), None, "Escape does not close a tool window");
}

#[trial]
fn an_app_modal_window_blocks_the_window_under_it_and_closes_on_escape() {
    let (mut fixture, seen) = rig(Some(Backdrop::None));
    press_under_then_escape(&mut fixture, &seen);
    assert_eq!(seen.presses.get(), 0, "the window under it took no press");
    assert!(seen.close.get().is_some(), "Escape asks to close an app-modal window");
}
