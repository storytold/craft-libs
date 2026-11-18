//! `TextOptions::submit_key`: which Enter chord submits a multi-line field and which one breaks the line.

use craft_ui::contract::{Bind, Show as _};
use craft_ui::primitives::SubmitKey;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Key, Modifiers};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Types "Hi" into a three-line field that submits on `submit_key`, then presses Enter with `modifiers`, rerunning
/// the pass of the press inside its frame. Gives the bound text and how many passes reported `submitted`.
fn press_enter(submit_key: SubmitKey, modifiers: Modifiers) -> (String, u32) {
    let text = Rc::new(RefCell::new(String::new()));
    let submits = Rc::new(Cell::new(0u32));
    let rerun = Rc::new(Cell::new(false));
    let (shown, counted, discard) = (text.clone(), submits.clone(), rerun.clone());
    let mut fixture = Fixture::new(move |ui| {
        if discard.take() {
            ui.ctx().request_discard("the trial reruns the pass of the Enter");
        }
        let out = TextField::new("Reply", Bind::bound(&mut *shown.borrow_mut())).lines(3).submits().submit_key(submit_key).show(ui);
        counted.set(counted.get() + u32::from(out.submitted));
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("Hi");
    fixture.idle(2);
    assert_eq!(*text.borrow(), "Hi", "the field takes the typing");
    submits.set(0);
    rerun.set(true);
    fixture.key(Key::Enter, modifiers);
    fixture.idle(2);
    let typed = text.borrow().clone();
    (typed, submits.get())
}

/// Under `SubmitKey::Enter` plain, Cmd and Ctrl+Enter submit and leave the text as it was, and Shift+Enter breaks the
/// line once without submitting; under the default `CommandEnter` plain Enter breaks the line and Cmd+Enter submits.
/// The pass of each Enter runs twice in its frame: the second applies none of the first's input, so nothing submits
/// or breaks twice.
#[trial]
fn enter_submits_and_shift_enter_breaks_a_line_under_the_enter_key() {
    for (chord, modifiers) in [("Enter", Modifiers::NONE), ("Cmd+Enter", Modifiers::COMMAND), ("Ctrl+Enter", Modifiers::CTRL)] {
        let (text, submits) = press_enter(SubmitKey::Enter, modifiers);
        assert_eq!(submits, 1, "{chord} submits once under the Enter key");
        assert_eq!(text, "Hi", "{chord} under the Enter key leaves the text unchanged");
    }
    let (text, submits) = press_enter(SubmitKey::Enter, Modifiers::SHIFT);
    assert_eq!(submits, 0, "Shift+Enter does not submit under the Enter key");
    assert_eq!(text, "Hi\n", "Shift+Enter breaks the line once under the Enter key");

    let (text, submits) = press_enter(SubmitKey::CommandEnter, Modifiers::NONE);
    assert_eq!(submits, 0, "plain Enter does not submit under the default key");
    assert_eq!(text, "Hi\n", "plain Enter breaks the line once under the default key");
    let (text, submits) = press_enter(SubmitKey::default(), Modifiers::COMMAND);
    assert_eq!(submits, 1, "Cmd+Enter submits once under the default key");
    assert_eq!(text, "Hi", "Cmd+Enter under the default key leaves the text unchanged");
}
