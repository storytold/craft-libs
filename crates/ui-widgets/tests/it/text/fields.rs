//! Typing runs, submit and escape, undo, completion, the number field and the caret blink.

use super::kit::Rig;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::primitives::{Completer, Completion};
use craft_ui::testing::input::key_press;
use craft_ui::testing::text::{caret, painted};
use craft_ui::testing::trial;
use craft_ui::text::{NumberField, TextField};
use egui::{Event, Key, Modifiers};
use std::ops::Range;

struct Prefixes;

impl Completer for Prefixes {
    fn complete(&self, text: &str, caret: usize, out: &mut Vec<Completion>) -> Option<Range<usize>> {
        let before = text.get(..caret)?;
        let hits: Vec<_> = ["alpha", "alps", "beta"].into_iter().filter(|word| !before.is_empty() && word.starts_with(before)).collect();
        out.extend(hits.iter().map(|word| Completion::new(*word, *word)));
        (!hits.is_empty()).then_some(0..caret)
    }
}

/// The completion list gets Up, Down and Tab from the field while it is open, and the caret does not move.
#[trial]
fn completion_list_takes_arrows_and_tab_while_open_and_the_caret_does_not_move() {
    let mut rig = Rig::new("");
    rig.focus();
    let frame = |rig: &mut Rig, events: Vec<Event>| {
        let (text, seen) = (&mut rig.text, &mut rig.seen);
        rig.frames.frame(events, |ui| {
            let out = TextField::new("Name", Bind::bound(&mut *text)).complete(Prefixes).id_salt("name").show(ui);
            seen.rect = out.response.rect;
            seen.id = out.response.id;
        });
    };
    frame(&mut rig, vec![Event::Text("al".into())]);
    for _ in 0..4 {
        frame(&mut rig, vec![]);
    }
    let shapes = {
        let (text, seen) = (&mut rig.text, &mut rig.seen);
        rig.frames.frame_shapes(|ui| {
            let out = TextField::new("Name", Bind::bound(&mut *text)).complete(Prefixes).id_salt("name").show(ui);
            seen.id = out.response.id;
        })
    };
    let shown = painted(&shapes);
    assert!(shown.iter().any(|t| t == "alpha") && shown.iter().any(|t| t == "alps"), "typing opens the list: {shown:?}");
    let widget = rig.widget();
    let before = caret(&rig.frames.ctx, widget);
    for key in [Key::ArrowDown, Key::ArrowDown, Key::ArrowUp, Key::ArrowDown] {
        frame(&mut rig, vec![key_press(key, Modifiers::NONE)]);
        frame(&mut rig, vec![]);
    }
    assert_eq!(caret(&rig.frames.ctx, widget), before, "Up and Down go to the list and do not also move the caret");
    let focus = craft_ui::testing::focus::egui_focused(&rig.frames.ctx);
    frame(&mut rig, vec![key_press(Key::Tab, Modifiers::NONE)]);
    for _ in 0..4 {
        frame(&mut rig, vec![]);
    }
    assert_eq!(craft_ui::testing::focus::egui_focused(&rig.frames.ctx), focus, "Tab went to the list, not to the next stop");
    assert_eq!(rig.text, "alps", "Tab accepted the highlighted candidate");
}

/// A value set from outside while a draft is open ends the run: the field shows the new value at once and Enter
/// commits that value, not the draft.
#[trial]
fn a_value_set_outside_ends_the_typing_run() {
    let value = std::rc::Rc::new(std::cell::RefCell::new(Some(1.0f64)));
    let shown = value.clone();
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
        NumberField::new("Size", Bind::bound(&mut *shown.borrow_mut())).show(ui);
    });
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.type_text("12");
    fixture.idle(1);
    *value.borrow_mut() = Some(7.0);
    fixture.idle(1);
    assert!(fixture.texts().iter().any(|run| run.text.contains('7')), "the new value shows in the next pass");
    assert!(fixture.texts().iter().all(|run| !run.text.contains("12")), "the stale draft is gone");
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(3);
    const SET: f64 = 7.0;
    assert_eq!(*value.borrow(), Some(SET), "Enter commits the value that was set, not the draft");
}
