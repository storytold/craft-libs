//! Trials of the dialog shells' layer behaviour: initial focus, Escape and a closed dialog's keys; and of the prompts
//! on the dialog host: the default answer and the Apply to All box.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::{
    BlockingOverlay, Dialog, DialogHost, DialogResult, PromptChoice, RecoveredDocument, RecoveryAnswer, RecoveryDialog, StackDialog, StandardPrompt,
    UnsavedChangesDialog,
};
use crate::buttons::Button;
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Role, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::layout_components::Row;
use crate::runtime::overlay::CloseReason;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::input::key_press;
use crate::testing::relations::{apart_above, grows_with, inside, off_foot, same_column};
use crate::testing::{Fixture, Frames, trial};
use crate::text::TextField;
use craft_quit::{CloseAnswer, CloseScope};
use egui::{Key, Modifiers, Rect, Ui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

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
fn a_dialog_with_unchanged_fields_shows_close_in_place_of_ok_and_cancel() {
    let run = |changed: bool| {
        let seen = Rc::new(Cell::new((None, None)));
        let sink = seen.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut open = true;
            let shown = Dialog::new("Properties")
                .opened(Bind::bound(&mut open))
                .close_unchanged(true)
                .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
                .show(ui, |_, cx| Dialog::report_changed(cx, changed));
            if shown.output.closed.is_some() {
                sink.set((shown.output.closed, shown.output.picked));
            }
        });
        fixture.idle(30);
        // The title row's close action carries the same name; the footer's button is the last of them.
        let closes: Vec<_> =
            fixture.by_role(Role::BUTTON).into_iter().filter(|target| fixture.tree().name_of(target.node) == Some("Close")).collect();
        let labels = (fixture.by_label("OK").is_some(), fixture.by_label("Cancel").is_some(), closes.len() > 1);
        if let Some(close) = closes.last().filter(|_| !changed) {
            fixture.click(close);
        }
        (labels, seen.get())
    };
    assert_eq!(run(false), ((false, false, true), (Some(CloseReason::Requested), Some(1))), "unchanged: Close, which answers as the Cancel button");
    assert_eq!(run(true).0, (true, true, false), "changed: OK and Cancel");
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

/// The settled bounds of the dialog "Options" and of its text, with `text` over a row of `options` buttons.
fn dialog_and_text(text: &str, options: usize) -> (Rect, Rect) {
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let mut open = true;
        Dialog::new("Options").opened(Bind::bound(&mut open)).buttons([FooterButton::new("OK", DialogRole::Accept)]).show(ui, |ui, _| {
            Text::new(text.to_owned()).show(ui);
            if options > 0 {
                Row::new().show(ui, |ui, _| {
                    for at in 0..options {
                        Button::new(format!("Option {at}")).show(ui);
                    }
                });
            }
        });
    };
    frames.idle(30, &mut ui);
    let tree = frames.tree();
    let rect = |role: Role, name: &str| {
        let key = tree.keys().into_iter().find(|key| tree.role_of(*key) == Some(role) && tree.name_of(*key) == Some(name));
        key.and_then(|key| tree.bounds_of(key))
    };
    match (rect(Role::DIALOG, "Options"), rect(Role::LABEL, text)) {
        (Some(dialog), Some(text)) => (dialog, text),
        shown => panic!("the dialog and its text are shown: {shown:?}"),
    }
}

/// A dialog opens at its least width, however short its content; a paragraph wraps at the dialog's wrap width even
/// where the window has room for it on one line; content that cannot wrap widens the dialog past that width.
#[trial]
fn a_dialogs_paragraphs_wrap_at_its_width_and_wider_content_widens_it() {
    let (short, word) = dialog_and_text("Sure?", 0);
    assert!(short.width() > 4.0 * word.width(), "a one-word dialog opens at the least width, not at its word: {word:?} {short:?}");
    let paragraph = "The document has changes from an earlier run that were saved automatically five minutes ago.";
    let (long, text) = dialog_and_text(paragraph, 0);
    assert!(text.height() > 1.5 * word.height(), "the paragraph wraps although the window could hold it on one line: {word:?} {text:?}");
    assert!(text.max.x <= long.max.x, "the paragraph stays inside the dialog: {text:?} {long:?}");
    let (wide, _) = dialog_and_text("Sure?", 8);
    assert!(wide.width() > long.width() + 1.0, "a row wider than the wrap width widens the dialog: {long:?} {wide:?}");
}

/// The dialog chrome every dialog shares: the body starts apart from the title row's rule and ends apart from the
/// footer, and the buttons stay inside the dialog, off its foot.
#[trial]
fn a_dialogs_body_stands_apart_from_its_header_and_its_footer() {
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let (mut open, mut snap) = (true, false);
        Dialog::new("Options")
            .opened(Bind::bound(&mut open))
            .buttons([FooterButton::new("Cancel", DialogRole::Cancel), FooterButton::new("OK", DialogRole::Accept)])
            .show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut snap)).label("Snap to the grid").show(ui);
            });
    };
    frames.idle(30, &mut ui);
    let tree = frames.tree();
    let rect = |role: Role, name: &str| {
        let key = tree.keys().into_iter().find(|key| tree.role_of(*key) == Some(role) && tree.name_of(*key) == Some(name));
        key.and_then(|key| tree.bounds_of(key))
    };
    let [Some(dialog), Some(title), Some(body), Some(ok)] =
        [rect(Role::DIALOG, "Options"), rect(Role::HEADING, "Options"), rect(Role::CHECK_BOX, "Snap to the grid"), rect(Role::BUTTON, "OK")]
    else {
        panic!("every part is shown");
    };
    assert!(apart_above(title, body), "a space separates the body from the title row's rule: {title:?} {body:?}");
    assert!(apart_above(body, ok), "a space separates the body from the footer: {body:?} {ok:?}");
    assert!(inside(ok, dialog) && off_foot(ok, dialog), "the buttons stay off the dialog's foot: {ok:?} {dialog:?}");
}

#[trial]
fn side_buttons_stand_in_one_column_beside_the_body() {
    let mut frames = Frames::new();
    let mut ui = |ui: &mut Ui| {
        let (mut open, mut snap, mut preview) = (true, false, true);
        Dialog::new("Options")
            .opened(Bind::bound(&mut open))
            .side_buttons(true)
            .buttons([
                FooterButton::new("Add to Swatches", DialogRole::Other),
                FooterButton::new("Cancel", DialogRole::Cancel),
                FooterButton::new("OK", DialogRole::Accept),
            ])
            .footer(|ui, _| {
                Checkbox::new(Bind::bound(&mut preview)).label("Preview").show(ui);
            })
            .show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut snap)).label("Snap to the grid").show(ui);
            });
    };
    frames.idle(30, &mut ui);
    let tree = frames.tree();
    let rect = |name: &str| {
        let key = tree.keys().into_iter().find(|key| tree.name_of(*key) == Some(name));
        key.and_then(|key| tree.bounds_of(key))
    };
    // The body's top is its check box's cell; its label is centred on the box, below the row's top.
    let body = tree
        .keys()
        .into_iter()
        .find(|key| tree.role_of(*key) == Some(Role::CHECK_BOX) && tree.name_of(*key) == Some("Snap to the grid"))
        .and_then(|key| tree.bounds_of(key));
    let [Some(ok), Some(cancel), Some(other), Some(preview), Some(body)] =
        [rect("OK"), rect("Cancel"), rect("Add to Swatches"), rect("Preview"), body]
    else {
        panic!("every part is shown");
    };
    assert!(ok.max.y <= cancel.min.y && cancel.max.y <= other.min.y, "OK, Cancel, then the other button, top to bottom");
    for button in [cancel, other] {
        assert!(same_column(ok, button), "one column, one width: {ok:?} {button:?}");
    }
    assert!(ok.min.x >= body.max.x, "the column stands past the body's end edge");
    assert!(grows_with(body.min.y, ok.min.y) && ok.min.y <= body.center().y, "the column starts at the body's top: {ok:?} {body:?}");
    assert!(preview.min.y >= other.max.y, "the footer content sits under the buttons");
}

#[trial]
fn a_dialog_opens_on_its_first_field_else_its_default_button() {
    let focus_on_open = |field: bool, buttons: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let (mut open, mut name, mut on) = (true, String::new(), false);
            let mut dialog = Dialog::new("Options").opened(Bind::bound(&mut open));
            if buttons {
                dialog = dialog.buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)]);
            }
            dialog.show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut on)).label("Snap").show(ui);
                if field {
                    TextField::new("Name", Bind::bound(&mut name)).show(ui);
                }
            });
        });
        fixture.idle(30);
        let focused = crate::testing::access::focused_key(fixture.tree());
        ["Name", "OK", "Snap"].into_iter().find(|label| fixture.by_label(label).is_some_and(|target| target.node == focused))
    };
    assert_eq!(focus_on_open(true, true), Some("Name"), "the first field takes the focus though a check box comes first");
    assert_eq!(focus_on_open(false, true), Some("OK"), "without a field the default button takes it");
    assert_eq!(focus_on_open(false, false), None, "without either nothing takes it");
}

/// A dialog host showing `dialog` in a fixture, once the dialog settled.
fn hosted<D: StackDialog>(dialog: D) -> (Rc<RefCell<DialogHost>>, Fixture) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    host.borrow_mut().open(dialog);
    fixture.idle(30);
    (host, fixture)
}

/// The choice a hosted `prompt` answers to `key`; Cancel when the host reports the dialog cancelled.
fn prompt_answer_to(prompt: StandardPrompt, key: Key) -> Option<PromptChoice> {
    let (host, mut fixture) = hosted(prompt);
    fixture.key(key, Modifiers::NONE).idle(2);
    let result = host.borrow_mut().take::<StandardPrompt>();
    result.map(|result| match result {
        DialogResult::Ok(answer) => answer.choice,
        DialogResult::Cancel => PromptChoice::Cancel,
    })
}

/// A prompt asking to open a link: Open (Proceed) and Cancel.
fn link_prompt() -> StandardPrompt {
    StandardPrompt::custom("Open the link “https://example.com”?").button(PromptChoice::Proceed, "Open").button(PromptChoice::Cancel, "Cancel")
}

#[trial]
fn enter_answers_a_prompts_chosen_default() {
    let safe = || link_prompt().default_choice(PromptChoice::Cancel);
    assert_eq!(prompt_answer_to(safe(), Key::Enter), Some(PromptChoice::Cancel), "Enter answers the chosen default, so it opens no link");
    assert_eq!(prompt_answer_to(safe(), Key::Escape), Some(PromptChoice::Cancel), "Escape still cancels");
    assert_eq!(prompt_answer_to(link_prompt(), Key::Enter), Some(PromptChoice::Proceed), "without a chosen default Enter proceeds");
    let missing = link_prompt().default_choice(PromptChoice::Alternate);
    assert_eq!(prompt_answer_to(missing, Key::Enter), Some(PromptChoice::Proceed), "a default with no button leaves Proceed the default");
}

/// Shows `dialog` on a dialog host, ticks its "Apply to All" box when `tick` and the box shows, and presses the first
/// button of `accept` that is shown. Whether the box showed, and the host's result.
fn ticked_and_accepted<D: StackDialog>(dialog: D, tick: bool, accept: &[&str]) -> (bool, Option<DialogResult<D::Result>>) {
    let (host, mut fixture) = hosted(dialog);
    let offered = fixture.by_label("Apply to All");
    if let (true, Some(apply)) = (tick, &offered) {
        fixture.click(apply).idle(2);
    }
    let button = accept.iter().find_map(|label| fixture.by_label(label)).expect("the accepting button is shown");
    fixture.click(&button).idle(2);
    let result = host.borrow_mut().take::<D>();
    (offered.is_some(), result)
}

#[trial]
fn apply_to_all_shows_only_with_several_documents_and_rides_the_answer() {
    // Save on macOS, Yes elsewhere.
    let save = ["Save", "Yes"];
    let unsaved = |remaining: usize, tick: bool| {
        let (offered, result) = ticked_and_accepted(UnsavedChangesDialog::new(CloseScope::Close, "poster.pdf").remaining(remaining), tick, &save);
        match result {
            Some(DialogResult::Ok(answer)) => (offered, answer.answer, answer.apply_to_all),
            other => panic!("the dialog answers Save: {other:?}"),
        }
    };
    assert_eq!(unsaved(1, true), (false, CloseAnswer::Save, false), "one document: no box, and the answer applies to it alone");
    assert_eq!(unsaved(3, true), (true, CloseAnswer::Save, true), "several documents: the ticked box rides the answer");
    assert_eq!(unsaved(3, false), (true, CloseAnswer::Save, false), "an unticked box leaves the answer to this document");
    let recovery = |remaining: usize, tick: bool| {
        let document = RecoveredDocument::new("poster", "poster.pdf", Duration::from_secs(300));
        let (offered, result) = ticked_and_accepted(RecoveryDialog::new(document).remaining(remaining), tick, &["Recover"]);
        match result {
            Some(DialogResult::Ok(choice)) => (offered, choice.answer, choice.apply_to_all),
            other => panic!("the dialog answers Recover: {other:?}"),
        }
    };
    assert_eq!(recovery(1, true), (false, RecoveryAnswer::Recover, false), "one document: no box");
    assert_eq!(recovery(3, true), (true, RecoveryAnswer::Recover, true), "several documents: the ticked box rides the answer");
}

/// A progress overlay shows a spinner beside its message on its card, is named by the message, and its Cancel makes
/// Escape close it.
#[trial]
fn a_progress_overlay_shows_a_spinner_and_its_message_and_escape_cancels_it() {
    let open = std::rc::Rc::new(Cell::new(true));
    let seen = std::rc::Rc::clone(&open);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = seen.get();
        BlockingOverlay::new("Working").opened(Bind::bound(&mut now)).message("Exporting 24 pages").cancel("Stop").show(ui, |_, _| {});
        seen.set(now);
    });
    fixture.settle();
    let spinner = std::any::type_name::<crate::display::Spinner>();
    assert!(!fixture.styles_of(spinner).is_empty(), "the card shows a spinner");
    assert!(fixture.texts().iter().any(|run| run.text == "Exporting 24 pages"), "the card shows the message");
    assert!(fixture.by_label("Exporting 24 pages").is_some(), "the message names the overlay");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    assert!(!open.get(), "Escape closed the overlay that has a Cancel button");
}
