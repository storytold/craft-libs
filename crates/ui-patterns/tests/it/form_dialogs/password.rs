//! The password dialog keeps the keyboard in its field and rates a new password as it is typed.

use craft_ui::patterns::form_dialogs::{PasswordDialog, PasswordPurpose};
use craft_ui::testing::{Fixture, trial};
use egui::Modifiers;
use std::cell::RefCell;
use std::rc::Rc;

fn focused_on(fixture: &Fixture, label: &str) -> bool {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    fixture.by_label(label).is_some_and(|target| target.node == focused)
}

#[trial]
fn the_reveal_button_hands_the_focus_back_to_the_field() {
    let mut fixture = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::open("poster.pdf")).show(ui);
    });
    fixture.idle(30);
    assert!(focused_on(&fixture, "Password"), "the dialog opens on the field");
    let reveal = fixture.by_label("Show password").expect("the reveal button is shown");
    fixture.click(&reveal).idle(4);
    assert!(fixture.by_label("Hide password").is_some(), "the press revealed the text");
    assert!(focused_on(&fixture, "Password"), "the field has the focus back, so typing goes on");
}

/// Enter in the field answers Open with the typed password; a new password answers only once both entries match.
#[trial]
fn enter_in_the_field_answers_while_the_entries_are_valid() {
    for (purpose, retyped, answers) in [
        (PasswordPurpose::open("poster.pdf"), None, true),
        (PasswordPurpose::Set, Some("other"), false),
        (PasswordPurpose::Set, Some("secret"), true),
    ] {
        let answer = Rc::new(RefCell::new(None));
        let seen = Rc::clone(&answer);
        let mut fixture = Fixture::new(move |ui| {
            let shown = PasswordDialog::new(purpose.clone()).show(ui);
            if let Some(given) = shown.answer {
                *seen.borrow_mut() = Some(given.secret.expose().to_owned());
            }
        });
        fixture.idle(30);
        fixture.type_text("secret").idle(2);
        if let Some(retyped) = retyped {
            let field = fixture.by_label("Re-type Password").expect("a new password is typed twice");
            fixture.click(&field).idle(2);
            fixture.type_text(retyped).idle(2);
        }
        fixture.key(egui::Key::Enter, Modifiers::NONE).idle(4);
        let expected = answers.then(|| "secret".to_owned());
        assert_eq!(*answer.borrow(), expected, "Enter after {retyped:?}");
    }
}

/// The strength meter's fill and the top of its range: the fill grows with the rating and Strong reaches the top.
fn meter(fixture: &Fixture) -> (f64, f64) {
    let meter = fixture.by_label("Password strength").expect("the strength meter shows once something is typed");
    let numeric = fixture.tree().numeric_of(meter.node).expect("the meter reports how full it is");
    (numeric.value.expect("the meter reports how full it is"), *numeric.range.end())
}

#[trial]
fn a_password_is_rated_by_its_length_and_its_character_classes() {
    let mut fixture = Fixture::new(|ui| {
        let _ = PasswordDialog::new(PasswordPurpose::Set).show(ui);
    });
    fixture.idle(30);
    assert!(focused_on(&fixture, "Type Password"), "the dialog opens on the first field");
    assert!(fixture.by_label("Password strength").is_none(), "an empty field has no rating");
    // Each step adds characters, so the text only grows longer and gains classes.
    fixture.type_text("abc").idle(2);
    let (short, top) = meter(&fixture);
    assert!(short < top, "three lower-case letters are not the top rating");
    fixture.type_text("DEFGH").idle(2);
    let (mixed, _) = meter(&fixture);
    assert!(mixed >= short, "eight characters over two classes rate no lower than three over one");
    fixture.type_text("12!x").idle(2);
    let (long, _) = meter(&fixture);
    assert!(long >= mixed, "more length and classes never lower the rating");
    assert!(long >= top, "twelve characters over three classes fill the meter");
    assert!(short < long, "the rating rose from weak to strong");
}
