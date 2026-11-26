//! macOS Control-click: the raw-input step turns a Control press into a secondary press until its release.

use craft_host::target::Os;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

fn button(pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos: Pos2::new(50.0, 50.0), button: PointerButton::Primary, pressed, modifiers }
}

/// (secondary clicked, clicked, secondary down, primary down) over the frames since the last read.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Seen {
    secondary: bool,
    primary: bool,
    secondary_down: bool,
    primary_down: bool,
}

fn fixture(os: Os) -> (Fixture, Rc<Cell<Seen>>) {
    let seen = Rc::new(Cell::new(Seen::default()));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let r = ui.allocate_rect(Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)), egui::Sense::click());
        let (secondary_down, primary_down) = ui.input(|i| (i.pointer.secondary_down(), i.pointer.primary_down()));
        let was = out.get();
        out.set(Seen { secondary: was.secondary || r.secondary_clicked(), primary: was.primary || r.clicked(), secondary_down, primary_down });
    });
    fixture.os(os);
    fixture.settle();
    fixture.frame(vec![Event::PointerMoved(Pos2::new(50.0, 50.0))]);
    seen.set(Seen::default());
    (fixture, seen)
}

#[trial]
fn a_macos_control_click_is_a_secondary_click_until_its_release() {
    // Control released before the button: the click stays secondary.
    let (mut fx, seen) = fixture(Os::Mac);
    fx.frame(vec![button(true, Modifiers::CTRL)]);
    assert!(seen.get().secondary_down && !seen.get().primary_down, "the Control press is a secondary press");
    fx.frame(vec![button(false, Modifiers::NONE)]);
    fx.frame(Vec::new());
    assert!(seen.get().secondary && !seen.get().primary, "it ends as a secondary click: {:?}", seen.get());

    // Off macOS, and Control with Option or Command on macOS, the press stays primary.
    for (os, mods) in [
        (Os::Linux, Modifiers::CTRL),
        (Os::Windows, Modifiers::CTRL),
        (Os::Mac, Modifiers::CTRL | Modifiers::ALT),
        (Os::Mac, Modifiers::CTRL | Modifiers::COMMAND),
    ] {
        let (mut fx, seen) = fixture(os);
        fx.frame(vec![button(true, mods)]);
        fx.frame(vec![button(false, mods)]);
        fx.frame(Vec::new());
        assert!(seen.get().primary && !seen.get().secondary, "{os:?} {mods:?} stays primary: {:?}", seen.get());
    }

    // A Control drag out of the window, a focus loss and back: the release ends the secondary press, and the next
    // plain click is primary.
    let (mut fx, seen) = fixture(Os::Mac);
    fx.frame(vec![button(true, Modifiers::CTRL)]);
    fx.frame(vec![Event::PointerGone]);
    fx.frame(vec![Event::WindowFocused(false)]);
    fx.frame(vec![Event::PointerMoved(Pos2::new(60.0, 60.0))]);
    fx.frame(vec![button(false, Modifiers::NONE)]);
    assert!(!seen.get().secondary_down && !seen.get().primary_down, "no button is left down: {:?}", seen.get());
    seen.set(Seen::default());
    fx.frame(vec![button(true, Modifiers::NONE)]);
    fx.frame(vec![button(false, Modifiers::NONE)]);
    fx.frame(Vec::new());
    assert!(seen.get().primary && !seen.get().secondary, "the next plain click is primary: {:?}", seen.get());
}
