//! The frame loop: installed options, the one-press rule and the per-frame output.

use craft_ui::testing::{Frames, trial};
use egui::{Event, Modifiers, PointerButton, Pos2};

const AT: Pos2 = Pos2::new(20.0, 12.0);

fn press(pressed: bool) -> Event {
    Event::PointerButton { pos: AT, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn kinds(events: &[Event]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            Event::PointerButton { pressed: true, .. } => "press",
            Event::PointerButton { pressed: false, .. } => "release",
            Event::PointerMoved(_) => "move",
            _ => "other",
        })
        .collect()
}

/// H0: a pass carries at most one press, as its last pointer event; later events lead the next frame, and a
/// discard pass receives none of the held events.
#[trial]
fn one_press_per_pass_and_a_discard_pass_gets_no_held_events() {
    let mut frames = Frames::new();
    let seen = std::cell::RefCell::new(Vec::<Vec<&'static str>>::new());
    let run = |frames: &mut Frames, events: Vec<Event>, discard_first: bool| {
        seen.borrow_mut().clear();
        let mut first = true;
        frames.frame(events, |ui| {
            seen.borrow_mut().push(kinds(&ui.input(|i| i.events.clone())));
            if discard_first && std::mem::take(&mut first) {
                ui.ctx().request_discard("test");
            }
        });
        seen.borrow().clone()
    };
    assert_eq!(run(&mut frames, vec![Event::PointerMoved(AT), press(true), press(false)], false), vec![vec!["move", "press"]]);
    assert_eq!(run(&mut frames, vec![], false), vec![vec!["release"]], "the release comes one pass after its press");
    assert_eq!(
        run(&mut frames, vec![press(true), press(false), press(true)], true),
        vec![vec!["press"], vec![]],
        "the discard pass receives no held events"
    );
    assert_eq!(run(&mut frames, vec![], false), vec![vec!["release", "press"]], "the held events lead the next frame, and its own press is the last");
    assert_eq!(run(&mut frames, vec![], false), vec![Vec::<&str>::new()]);
}
