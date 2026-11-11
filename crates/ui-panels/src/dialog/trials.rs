//! Trials of the dialog shells' layer behaviour: initial focus, Escape and a closed dialog's keys.

use super::{BlockingOverlay, Dialog};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::runtime::overlay::CloseReason;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::input::key_press;
use crate::testing::{Fixture, Frames, trial};
use crate::text::TextField;
use egui::{Key, Modifiers, Ui};
use std::cell::Cell;

#[trial]
fn escape_closes_a_cancellable_overlay_with_nothing_to_focus() {
    let open = Cell::new(true);
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let mut now = open.get();
        BlockingOverlay::new("Working").opened(Bind::bound(&mut now)).cancellable(true).show(ui, |_, _| {});
        open.set(now);
    };
    frames.idle(30, &mut ui);
    assert!(open.get(), "the overlay is open");
    frames.frame(vec![key_press(Key::Escape, Modifiers::NONE)], &mut ui);
    frames.idle(2, &mut ui);
    assert!(!open.get(), "Escape closed the overlay though nothing inside takes the focus");
}

#[trial]
fn a_footer_button_closes_the_dialog_by_its_role() {
    let run = |label: &str| {
        let seen = std::rc::Rc::new(Cell::new((None, None)));
        let sink = seen.clone();
        let mut fixture = Fixture::new(move |ui| {
            let (mut open, mut name) = (true, String::new());
            let shown = Dialog::new("Options")
                .opened(Bind::bound(&mut open))
                .buttons([FooterButton::new("Delete", DialogRole::Destructive), FooterButton::new("Cancel", DialogRole::Cancel)])
                .show(ui, |ui, _| {
                    TextField::new("Name", Bind::bound(&mut name)).show(ui);
                });
            if shown.output.closed.is_some() {
                sink.set((shown.output.closed, shown.output.picked));
            }
        });
        fixture.idle(30);
        let button = fixture.by_label(label);
        assert!(button.is_some(), "the button is shown");
        if let Some(button) = button {
            fixture.click(&button);
        }
        seen.get()
    };
    assert_eq!(run("Cancel"), (Some(CloseReason::Requested), Some(1)), "Cancel closes as requested");
    assert_eq!(run("Delete"), (Some(CloseReason::Activated), Some(0)), "any other button closes as an outcome");
}

#[trial]
fn a_blocking_overlays_buttons_keep_their_place_after_it_settled() {
    let mut fixture = Fixture::new(|ui| {
        let mut open = true;
        BlockingOverlay::new("Working").opened(Bind::bound(&mut open)).card(true).cancel("Stop").show(ui, |_, _| {});
    });
    fixture.settle();
    let first = fixture.by_label("Stop").map(|button| button.rect);
    assert!(first.is_some(), "the cancel button is shown");
    fixture.idle(10);
    assert_eq!(fixture.by_label("Stop").map(|button| button.rect), first, "the overlay did not move or resize after it settled");
}

/// The width of each footer button by name, after the dialog settled, with a body of `body` text.
fn button_widths(body: &str, names: [&'static str; 3]) -> [f32; 3] {
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let (mut open, mut snap) = (true, false);
        Dialog::new("Options")
            .opened(Bind::bound(&mut open))
            .buttons(
                names.into_iter().zip([DialogRole::Other, DialogRole::Cancel, DialogRole::Accept]).map(|(name, role)| FooterButton::new(name, role)),
            )
            .show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut snap)).label(body.to_owned()).show(ui);
            });
    };
    frames.idle(30, &mut ui);
    let tree = frames.tree();
    names.map(|name| {
        let key = tree.keys().into_iter().find(|key| tree.name_of(*key) == Some(name));
        key.and_then(|key| tree.bounds_of(key)).map_or(0.0, |bounds| bounds.width())
    })
}

#[trial]
fn a_dialog_widens_for_a_footer_wider_than_its_body() {
    let names = ["Discard all changes", "Keep editing", "Save as a new version"];
    let short = button_widths("Sure?", names);
    let long = button_widths(&"A body wider than the three buttons together. ".repeat(6), names);
    assert!(short.iter().all(|width| *width > 0.0), "each button is shown: {short:?}");
    assert_eq!(short, long, "a short body does not squeeze the buttons");
}
